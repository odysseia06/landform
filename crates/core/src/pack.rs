//! The `.lfpk` pack: a game-ready container of real elevation data, quantised to 16 bits.
//!
//! Little-endian layout: magic `LFPK`; `version: u32 = 1`; `width: u32`; `height: u32`;
//! `cell_size_m: f32`; `height_scale: f32`; `height_offset: f32`; `nodata: u16`; `source_id`
//! and `attribution` as a `u16` length plus UTF-8; then `width * height` `u16` values,
//! row-major, row 0 north. `height_m = height_offset + height_scale * value`.

use crate::error::LandformError;
use crate::grid::Grid;
use crate::source::{ElevationSource, HeightBlock, RectM, SourceInfo};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The sample value that means "no data".
pub const PACK_NODATA: u16 = u16::MAX;

const MAGIC: &[u8; 4] = b"LFPK";
const VERSION: u32 = 1;
/// The largest quantised height a pack stores; `PACK_NODATA` lies above it.
pub const PACK_MAX_STORED: u16 = PACK_NODATA - 1;
const BYTES_PER_SAMPLE: usize = 2;

/// What a pack's header records.
#[derive(Debug, Clone, PartialEq)]
pub struct PackHeader {
    pub width: u32,
    pub height: u32,
    pub cell_size_m: f32,
    pub height_scale: f32,
    pub height_offset: f32,
    pub source_id: String,
    pub attribution: String,
}

impl PackHeader {
    /// The height in metres of the largest value the pack can store.
    pub fn highest_m(&self) -> f32 {
        self.height_offset + self.height_scale * f32::from(PACK_MAX_STORED)
    }
}

fn corrupt(path: &Path, reason: &str) -> LandformError {
    LandformError::PackCorrupt { path: path.to_path_buf(), reason: reason.to_string() }
}

/// Writes `heights` as a pack. The offset is the minimum finite height and the scale spans
/// the finite range over 65534 steps; `NaN` and infinite samples become `PACK_NODATA`.
pub fn write_pack(
    path: &Path,
    source_id: &str,
    attribution: &str,
    heights: &Grid<f32>,
) -> Result<PackHeader, LandformError> {
    let too_big = || LandformError::Io(format!("{}: the grid or its text is too large for a pack", path.display()));
    if heights.width == 0 || heights.height == 0 {
        return Err(LandformError::Io(format!("{}: the grid is empty", path.display())));
    }
    let width = u32::try_from(heights.width).map_err(|_| too_big())?;
    let height = u32::try_from(heights.height).map_err(|_| too_big())?;
    let id_len = u16::try_from(source_id.len()).map_err(|_| too_big())?;
    let attribution_len = u16::try_from(attribution.len()).map_err(|_| too_big())?;

    let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
    for &v in heights.data.iter().filter(|v| v.is_finite()) {
        min = min.min(v);
        max = max.max(v);
    }
    let (offset, scale) = if min <= max {
        (min, ((f64::from(max) - f64::from(min)) / f64::from(PACK_MAX_STORED)) as f32)
    } else {
        (0.0, 0.0) // no finite sample at all
    };
    let header = PackHeader {
        width,
        height,
        cell_size_m: heights.cell_size,
        height_scale: scale,
        height_offset: offset,
        source_id: source_id.to_string(),
        attribution: attribution.to_string(),
    };

    let mut bytes = Vec::with_capacity(40 + source_id.len() + attribution.len() + heights.data.len() * BYTES_PER_SAMPLE);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&header.cell_size_m.to_le_bytes());
    bytes.extend_from_slice(&scale.to_le_bytes());
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&PACK_NODATA.to_le_bytes());
    bytes.extend_from_slice(&id_len.to_le_bytes());
    bytes.extend_from_slice(source_id.as_bytes());
    bytes.extend_from_slice(&attribution_len.to_le_bytes());
    bytes.extend_from_slice(attribution.as_bytes());
    for &v in &heights.data {
        let quantised = if !v.is_finite() {
            PACK_NODATA
        } else if scale > 0.0 {
            let steps = ((f64::from(v) - f64::from(offset)) / f64::from(scale)).round();
            steps.clamp(0.0, f64::from(PACK_MAX_STORED)) as u16
        } else {
            0 // every finite sample equals the offset
        };
        bytes.extend_from_slice(&quantised.to_le_bytes());
    }
    std::fs::write(path, bytes)
        .map_err(|e| LandformError::Io(format!("{}: {e}", path.display())))?;
    Ok(header)
}

/// Reads exactly `N` header bytes, or reports the header as truncated.
fn take<const N: usize>(file: &mut File, path: &Path) -> Result<[u8; N], LandformError> {
    let mut buffer = [0u8; N];
    file.read_exact(&mut buffer).map_err(|_| corrupt(path, "the header is truncated"))?;
    Ok(buffer)
}

fn take_text(file: &mut File, path: &Path, what: &str) -> Result<String, LandformError> {
    let len = usize::from(u16::from_le_bytes(take(file, path)?));
    let mut buffer = vec![0u8; len];
    file.read_exact(&mut buffer).map_err(|_| corrupt(path, "the header is truncated"))?;
    String::from_utf8(buffer).map_err(|_| corrupt(path, &format!("the {what} is not valid UTF-8")))
}

/// An elevation source backed by a pack file. Every `read` opens the file again and reads
/// only the rows and columns it needs, so the pack is never held in memory.
pub struct PackSource {
    path: PathBuf,
    header: PackHeader,
    info: SourceInfo,
    data_offset: u64,
}

impl PackSource {
    /// Opens a pack, checking the magic, the version and that the file length matches the header.
    pub fn open(path: &Path) -> Result<Self, LandformError> {
        let mut file = File::open(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => LandformError::PackNotFound(path.to_path_buf()),
            _ => corrupt(path, &format!("it cannot be opened ({e})")),
        })?;
        let file_len = file
            .metadata()
            .map_err(|e| corrupt(path, &format!("its size cannot be read ({e})")))?
            .len();

        if &take::<4>(&mut file, path)? != MAGIC {
            return Err(corrupt(path, "it does not start with the LFPK marker"));
        }
        let version = u32::from_le_bytes(take(&mut file, path)?);
        if version != VERSION {
            return Err(corrupt(path, &format!("version {version} is not supported")));
        }
        let width = u32::from_le_bytes(take(&mut file, path)?);
        let height = u32::from_le_bytes(take(&mut file, path)?);
        let cell_size_m = f32::from_le_bytes(take(&mut file, path)?);
        let height_scale = f32::from_le_bytes(take(&mut file, path)?);
        let height_offset = f32::from_le_bytes(take(&mut file, path)?);
        let nodata = u16::from_le_bytes(take(&mut file, path)?);
        let source_id = take_text(&mut file, path, "source id")?;
        let attribution = take_text(&mut file, path, "attribution")?;
        let data_offset = file
            .stream_position()
            .map_err(|e| corrupt(path, &format!("its position cannot be read ({e})")))?;

        if width == 0 || height == 0 {
            return Err(corrupt(path, "its size is zero"));
        }
        if !(cell_size_m.is_finite() && cell_size_m > 0.0) {
            return Err(corrupt(path, "its cell size is not a positive number"));
        }
        if !(height_scale.is_finite() && height_scale >= 0.0 && height_offset.is_finite()) {
            return Err(corrupt(path, "its height scale or offset is not valid"));
        }
        if nodata != PACK_NODATA {
            return Err(corrupt(path, "its no-data value is not supported"));
        }
        let expected_len = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|samples| samples.checked_mul(BYTES_PER_SAMPLE as u64))
            .and_then(|data_len| data_len.checked_add(data_offset))
            .ok_or_else(|| corrupt(path, "its size is not valid"))?;
        if file_len != expected_len {
            return Err(corrupt(
                path,
                &format!("it is {file_len} bytes long but its header describes {expected_len}"),
            ));
        }

        let cell = f64::from(cell_size_m);
        let info = SourceInfo {
            id: source_id.clone(),
            width_m: f64::from(width - 1) * cell,
            height_m: f64::from(height - 1) * cell,
            cell_size_m,
            attribution: attribution.clone(),
        };
        let header = PackHeader {
            width,
            height,
            cell_size_m,
            height_scale,
            height_offset,
            source_id,
            attribution,
        };
        Ok(Self { path: path.to_path_buf(), header, info, data_offset })
    }

    pub fn header(&self) -> &PackHeader {
        &self.header
    }

    /// The raw little-endian samples of `cols` x `rows` cells starting at the given cell.
    fn read_samples(&self, first_col: usize, first_row: usize, cols: usize, rows: usize) -> std::io::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        let pack_width = self.header.width as usize;
        let row_bytes = cols * BYTES_PER_SAMPLE;
        let mut bytes = vec![0u8; row_bytes * rows];
        for (row, chunk) in bytes.chunks_exact_mut(row_bytes).enumerate() {
            let sample = (first_row + row) * pack_width + first_col;
            file.seek(SeekFrom::Start(self.data_offset + (sample * BYTES_PER_SAMPLE) as u64))?;
            file.read_exact(chunk)?;
        }
        Ok(bytes)
    }
}

impl ElevationSource for PackSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError> {
        let inside = rect.x >= 0.0
            && rect.y >= 0.0
            && rect.width >= 0.0
            && rect.height >= 0.0
            && rect.x + rect.width <= self.info.width_m
            && rect.y + rect.height <= self.info.height_m;
        if !inside {
            return Err(LandformError::OutOfBounds);
        }
        let cell = f64::from(self.header.cell_size_m);
        // Snap the origin down to a whole cell so repeated reads line up; the clamp only
        // guards against rounding at the far edge.
        let first_col = (rect.x / cell).floor() as usize;
        let first_row = (rect.y / cell).floor() as usize;
        let last_col = (((rect.x + rect.width) / cell).ceil() as usize).min(self.header.width as usize - 1);
        let last_row = (((rect.y + rect.height) / cell).ceil() as usize).min(self.header.height as usize - 1);
        let (cols, rows) = (last_col - first_col + 1, last_row - first_row + 1);

        let bytes = self
            .read_samples(first_col, first_row, cols, rows)
            .map_err(|e| corrupt(&self.path, &format!("its samples cannot be read ({e})")))?;
        let mut grid = Grid::new(cols, rows, self.header.cell_size_m);
        let scale = f64::from(self.header.height_scale);
        let offset = f64::from(self.header.height_offset);
        for (slot, raw) in grid.data.iter_mut().zip(bytes.chunks_exact(BYTES_PER_SAMPLE)) {
            let value = u16::from_le_bytes([raw[0], raw[1]]);
            *slot = if value == PACK_NODATA { f32::NAN } else { (offset + scale * f64::from(value)) as f32 };
        }
        Ok(HeightBlock { origin_x: first_col as f64 * cell, origin_y: first_row as f64 * cell, grid })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::check_source_contract;
    use std::path::PathBuf;

    /// A unique path in the temp directory, removed when dropped.
    struct TempPack(PathBuf);

    impl TempPack {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("landform_{}_{name}.lfpk", std::process::id())))
        }
    }

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn ramp_grid() -> Grid<f32> {
        let mut g = Grid::new(300, 200, 10.0);
        for j in 0..200 {
            for i in 0..300 {
                g.set(i, j, 120.0 + (i * 7 + j * 3) as f32); // 120.0 ..= 2810.0
            }
        }
        for k in 0..5 {
            g.set(10 + k, 20, f32::NAN);
        }
        g
    }

    #[test]
    fn pack_round_trips_within_sixteen_bit_precision() {
        let path = TempPack::new("round_trip");
        let g = ramp_grid();
        let h = write_pack(&path.0, "test", "Test data", &g).unwrap();
        assert_eq!((h.width, h.height, h.cell_size_m), (300, 200, 10.0));
        let s = PackSource::open(&path.0).unwrap();
        assert_eq!(s.header().attribution, "Test data");
        let b = s.read(RectM { x: 0.0, y: 0.0, width: 2990.0, height: 1990.0 }).unwrap();
        let step = (2810.0 - 120.0) / 65534.0;
        assert!((b.grid.get(150, 100) - g.get(150, 100)).abs() <= step);
        assert!(b.grid.get(12, 20).is_nan());
        check_source_contract(&s);
    }

    #[test]
    fn partial_read_matches_the_same_cells() {
        let path = TempPack::new("partial");
        let g = ramp_grid();
        write_pack(&path.0, "test", "Test data", &g).unwrap();
        let s = PackSource::open(&path.0).unwrap();
        let b = s.read(RectM { x: 500.0, y: 300.0, width: 200.0, height: 100.0 }).unwrap();
        assert_eq!((b.grid.width, b.grid.height), (21, 11));
        let step = (2810.0 - 120.0) / 65534.0;
        let (col0, row0) = ((b.origin_x / 10.0) as usize, (b.origin_y / 10.0) as usize);
        for j in 0..b.grid.height {
            for i in 0..b.grid.width {
                let want = g.get(col0 + i, row0 + j);
                assert!((b.grid.get(i, j) - want).abs() <= step, "sample ({i}, {j})");
            }
        }
    }

    #[test]
    fn info_describes_point_samples() {
        let path = TempPack::new("info");
        write_pack(&path.0, "dem", "Some DEM", &ramp_grid()).unwrap();
        let s = PackSource::open(&path.0).unwrap();
        let info = s.info();
        assert_eq!(info.id, "dem");
        assert_eq!(info.attribution, "Some DEM");
        assert_eq!(info.cell_size_m, 10.0);
        assert_eq!((info.width_m, info.height_m), (2990.0, 1990.0));
    }

    #[test]
    fn flat_grid_and_all_nodata_grid_round_trip() {
        let path = TempPack::new("flat");
        let mut flat = Grid::new(4, 4, 5.0);
        flat.data.fill(42.5);
        write_pack(&path.0, "flat", "x", &flat).unwrap();
        let b = PackSource::open(&path.0)
            .unwrap()
            .read(RectM { x: 0.0, y: 0.0, width: 15.0, height: 15.0 })
            .unwrap();
        assert!(b.grid.data.iter().all(|&v| v == 42.5));

        let mut empty = Grid::new(4, 4, 5.0);
        empty.data.fill(f32::NAN);
        write_pack(&path.0, "empty", "x", &empty).unwrap();
        let b = PackSource::open(&path.0)
            .unwrap()
            .read(RectM { x: 0.0, y: 0.0, width: 15.0, height: 15.0 })
            .unwrap();
        assert!(b.grid.data.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn reads_outside_the_pack_are_out_of_bounds() {
        let path = TempPack::new("bounds");
        write_pack(&path.0, "test", "x", &ramp_grid()).unwrap();
        let s = PackSource::open(&path.0).unwrap();
        for rect in [
            RectM { x: -1.0, y: 0.0, width: 10.0, height: 10.0 },
            RectM { x: 2985.0, y: 0.0, width: 10.0, height: 10.0 },
            RectM { x: 0.0, y: 1985.0, width: 10.0, height: 10.0 },
            RectM { x: f64::NAN, y: 0.0, width: 10.0, height: 10.0 },
        ] {
            assert_eq!(s.read(rect).err(), Some(LandformError::OutOfBounds));
        }
    }

    #[test]
    fn empty_grids_are_refused_when_writing() {
        let path = TempPack::new("empty_grid");
        let g: Grid<f32> = Grid::new(0, 5, 10.0);
        assert!(matches!(write_pack(&path.0, "x", "x", &g), Err(LandformError::Io(_))));
    }

    #[test]
    fn missing_pack_is_reported() {
        assert!(matches!(
            PackSource::open(Path::new("no_such.lfpk")),
            Err(LandformError::PackNotFound(_))
        ));
    }

    #[test]
    fn oversized_header_dimensions_are_reported_not_overflowed() {
        let good = TempPack::new("overflow_good");
        write_pack(&good.0, "test", "Test data", &ramp_grid()).unwrap();
        let mut bytes = std::fs::read(&good.0).unwrap();
        bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        let p = TempPack::new("overflow_bad");
        std::fs::write(&p.0, bytes).unwrap();
        let err = PackSource::open(&p.0).err().expect("an impossible size must fail");
        assert!(matches!(err, LandformError::PackCorrupt { .. }), "{err}");
    }

    #[test]
    fn damaged_packs_are_reported() {
        let good = TempPack::new("damaged_good");
        write_pack(&good.0, "test", "Test data", &ramp_grid()).unwrap();
        let bytes = std::fs::read(&good.0).unwrap();

        let mut wrong_version = bytes.clone();
        wrong_version[4] = 2;
        let cases: [(&str, Vec<u8>); 5] = [
            ("zero", Vec::new()),
            ("magic", b"NOPE and then some more bytes".to_vec()),
            ("truncated", bytes[..bytes.len() - 100].to_vec()),
            ("version", wrong_version),
            ("header_cut", bytes[..20].to_vec()),
        ];
        for (name, content) in cases {
            let p = TempPack::new(&format!("damaged_{name}"));
            std::fs::write(&p.0, content).unwrap();
            let err = PackSource::open(&p.0).err().unwrap_or_else(|| panic!("{name} must fail"));
            assert!(matches!(err, LandformError::PackCorrupt { .. }), "{name}: {err}");
            let text = err.to_string();
            let file_name = p.0.file_name().unwrap().to_string_lossy().into_owned();
            assert!(text.contains("pack") && text.contains(&file_name), "{name}: {text}");
        }
    }

    #[test]
    fn highest_m_is_the_top_of_the_stored_range() {
        let path = TempPack::new("highest");
        let header = write_pack(&path.0, "ramp", "Test data", &ramp_grid()).unwrap();
        assert!((header.highest_m() - 2810.0).abs() < 0.01, "{}", header.highest_m());
        assert_eq!(PackSource::open(&path.0).unwrap().header().highest_m(), header.highest_m());
    }
}
