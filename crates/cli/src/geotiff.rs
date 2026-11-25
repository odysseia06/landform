//! Reading USGS elevation tiles: float32 GeoTIFF in geographic coordinates.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tiff::{ColorType, TiffError};

/// A raster in geographic coordinates (degrees), north-up, with one height per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct GeoRaster {
    /// Longitude of the outer west edge of column 0.
    pub west: f64,
    /// Latitude of the outer north edge of row 0.
    pub north: f64,
    /// Pixel width in degrees of longitude.
    pub px_lon: f64,
    /// Pixel height in degrees of latitude.
    pub px_lat: f64,
    pub width: usize,
    pub height: usize,
    /// The height value that marks a missing sample, when the file declares one.
    pub nodata: Option<f32>,
    /// Heights in metres, row-major, row 0 at the north.
    pub data: Vec<f32>,
}

impl GeoRaster {
    /// Bilinear height at a point in degrees: `None` when the point lies outside the raster,
    /// `Some(NaN)` when any of the four pixels used is missing.
    ///
    /// Pixel `(c, r)` has its value at its centre. In the outer half pixel the edge pixels
    /// are repeated, so the raster answers everywhere inside its outer edges.
    pub fn sample(&self, lon: f64, lat: f64) -> Option<f32> {
        let fx = (lon - self.west) / self.px_lon;
        let fy = (self.north - lat) / self.px_lat;
        // Written so that NaN coordinates fall through to `None`.
        if !(fx >= 0.0 && fx < self.width as f64 && fy >= 0.0 && fy < self.height as f64) {
            return None;
        }
        let (gx, gy) = (fx - 0.5, fy - 0.5);
        let (c0, r0) = (gx.floor(), gy.floor());
        let (tx, ty) = (gx - c0, gy - r0);
        let (last_c, last_r) = (self.width as i64 - 1, self.height as i64 - 1);
        let column = |c: f64| (c as i64).clamp(0, last_c) as usize;
        let row = |r: f64| (r as i64).clamp(0, last_r) as usize;
        let (c1, r1) = (column(c0 + 1.0), row(r0 + 1.0));
        let (c0, r0) = (column(c0), row(r0));

        let at = |c: usize, r: usize| self.data[r * self.width + c];
        let corners = [at(c0, r0), at(c1, r0), at(c0, r1), at(c1, r1)];
        if corners.iter().any(|&h| h.is_nan() || Some(h) == self.nodata) {
            return Some(f32::NAN);
        }
        let [a, b, c, d] = corners.map(f64::from);
        let north = a + (b - a) * tx;
        let south = c + (d - c) * tx;
        Some((north + (south - north) * ty) as f32)
    }
}

/// Reads a float32 GeoTIFF into memory.
///
/// Georeferencing comes from the pixel scale (33550) and tiepoint (33922) tags, the tiepoint
/// being the outer corner of its pixel; the no-data value comes from tag 42113. The raster-type
/// key is not read, so a file whose values refer to pixel corners is off by half a pixel.
pub fn read_geotiff(path: &Path) -> Result<GeoRaster, String> {
    let name = path.display();
    let file = File::open(path).map_err(|e| format!("{name}: {e}"))?;
    let mut decoder = Decoder::new(BufReader::new(file))
        .map_err(|e| format!("{name} is not a readable TIFF file: {e}"))?
        // A whole 1 degree tile is about 470 MB, above the default limit of 256 MiB.
        .with_limits(Limits::unlimited());
    let bad = |what: &str, e: TiffError| format!("{name}: {what}: {e}");

    let (width, height) = decoder.dimensions().map_err(|e| bad("cannot read the size", e))?;
    let color = decoder.colortype().map_err(|e| bad("cannot read the pixel type", e))?;
    if color != ColorType::Gray(32) {
        return Err(format!("{name}: the pixels must be one 32-bit float band, not {color:?}"));
    }

    let mut tag_values = |tag: Tag, wanted: usize| -> Result<Vec<f64>, String> {
        let number = tag.to_u16();
        let value = decoder
            .find_tag(tag)
            .map_err(|e| bad(&format!("cannot read tag {number}"), e))?
            .ok_or_else(|| format!("{name}: the GeoTIFF tag {number} ({tag:?}) is missing"))?;
        let values = value.into_f64_vec().map_err(|e| bad(&format!("cannot read tag {number}"), e))?;
        if values.len() < wanted {
            return Err(format!("{name}: the GeoTIFF tag {number} needs {wanted} values"));
        }
        Ok(values)
    };
    let scale = tag_values(Tag::ModelPixelScaleTag, 2)?;
    let tiepoint = tag_values(Tag::ModelTiepointTag, 6)?;
    let (px_lon, px_lat) = (scale[0], scale[1]);
    if !(px_lon > 0.0 && px_lat > 0.0) {
        return Err(format!("{name}: the pixel scale must be positive"));
    }
    // The tiepoint pairs raster position (i, j) with the geographic position (x, y).
    let west = tiepoint[3] - tiepoint[0] * px_lon;
    let north = tiepoint[4] + tiepoint[1] * px_lat;

    let nodata = match decoder
        .find_tag(Tag::GdalNodata)
        .map_err(|e| bad("cannot read tag 42113", e))?
    {
        None => None,
        Some(value) => {
            let text = value.into_string().map_err(|e| bad("cannot read tag 42113", e))?;
            let text = text.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            let parsed = text
                .parse::<f32>()
                .map_err(|_| format!("{name}: the no-data value {text:?} is not a number"))?;
            Some(parsed)
        }
    };

    let data = match decoder.read_image().map_err(|e| bad("cannot decode the pixels", e))? {
        DecodingResult::F32(data) => data,
        _ => return Err(format!("{name}: the pixels must be one 32-bit float band")),
    };
    Ok(GeoRaster {
        west,
        north,
        px_lon,
        px_lat,
        width: width as usize,
        height: height as usize,
        nodata,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tiff::encoder::{colortype, TiffEncoder};

    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 6;
    const NODATA_PIXEL: (usize, usize) = (3, 2);

    fn temp_tiff(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("landform_geotiff_{}_{name}.tif", std::process::id()))
    }

    /// Writes a tiny float32 TIFF; the georeferencing tags are optional.
    fn write_test_tiff(path: &Path, georeferenced: bool) {
        let mut data = Vec::new();
        for r in 0..HEIGHT as usize {
            for c in 0..WIDTH as usize {
                data.push(if (c, r) == NODATA_PIXEL { -999_999.0 } else { (r * 10 + c) as f32 });
            }
        }
        let mut encoder = TiffEncoder::new(File::create(path).unwrap()).unwrap();
        let mut image = encoder.new_image::<colortype::Gray32Float>(WIDTH, HEIGHT).unwrap();
        if georeferenced {
            let directory = image.encoder();
            directory.write_tag(Tag::ModelPixelScaleTag, &[0.5f64, 0.25, 0.0][..]).unwrap();
            directory
                .write_tag(Tag::ModelTiepointTag, &[0.0f64, 0.0, 0.0, -124.0, 48.0, 0.0][..])
                .unwrap();
            directory.write_tag(Tag::GdalNodata, "-999999").unwrap();
        }
        image.write_data(&data).unwrap();
    }

    #[test]
    fn reads_georeferencing_nodata_and_pixels() {
        let path = temp_tiff("read");
        write_test_tiff(&path, true);
        let raster = read_geotiff(&path).expect("the file reads");
        let _ = std::fs::remove_file(&path);

        assert_eq!((raster.width, raster.height), (8, 6));
        assert_eq!((raster.west, raster.north), (-124.0, 48.0));
        assert_eq!((raster.px_lon, raster.px_lat), (0.5, 0.25));
        assert_eq!(raster.nodata, Some(-999_999.0));
        assert_eq!(raster.data.len(), 48);
        assert_eq!(raster.data[0], 0.0);
        assert_eq!(raster.data[7], 7.0);
        assert_eq!(raster.data[5 * 8 + 4], 54.0);
        assert_eq!(raster.data[2 * 8 + 3], -999_999.0);
    }

    #[test]
    fn a_missing_tag_is_an_error_naming_the_file() {
        let path = temp_tiff("untagged");
        write_test_tiff(&path, false);
        let error = read_geotiff(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("landform_geotiff_") && error.contains("untagged"), "{error}");
        assert!(error.contains("33550"), "{error}");
    }

    #[test]
    fn a_missing_file_is_an_error_naming_the_file() {
        let path = temp_tiff("absent");
        let error = read_geotiff(&path).unwrap_err();
        assert!(error.contains("absent"), "{error}");
    }

    #[test]
    fn other_pixel_types_are_rejected() {
        let path = temp_tiff("integers");
        let mut encoder = TiffEncoder::new(File::create(&path).unwrap()).unwrap();
        encoder.write_image::<colortype::Gray16>(2, 2, &[1, 2, 3, 4]).unwrap();
        drop(encoder);
        let error = read_geotiff(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(error.contains("integers") && error.contains("32-bit float"), "{error}");
    }

    fn raster_2x2(nodata: Option<f32>, data: [f32; 4]) -> GeoRaster {
        // Pixel centres sit at lon -9.75 and -9.25, lat 49.75 and 49.25.
        GeoRaster {
            west: -10.0,
            north: 50.0,
            px_lon: 0.5,
            px_lat: 0.5,
            width: 2,
            height: 2,
            nodata,
            data: data.to_vec(),
        }
    }

    #[test]
    fn sampling_is_bilinear_between_pixel_centres() {
        let raster = raster_2x2(None, [0.0, 10.0, 20.0, 30.0]);
        assert_eq!(raster.sample(-9.75, 49.75), Some(0.0));
        assert_eq!(raster.sample(-9.25, 49.25), Some(30.0));
        assert_eq!(raster.sample(-9.5, 49.5), Some(15.0));
        // The outer half pixel repeats the edge value.
        assert_eq!(raster.sample(-9.99, 49.75), Some(0.0));
    }

    #[test]
    fn sampling_outside_is_none_and_touching_nodata_is_nan() {
        let raster = raster_2x2(Some(-9.0), [0.0, 10.0, -9.0, 30.0]);
        assert_eq!(raster.sample(-10.1, 49.5), None);
        assert_eq!(raster.sample(-9.5, 50.1), None);
        assert_eq!(raster.sample(-8.9, 49.5), None);
        assert_eq!(raster.sample(f64::NAN, 49.5), None);
        assert!(raster.sample(-9.5, 49.5).unwrap().is_nan());
        // In the outer corner only pixel (0, 0) is used, so the missing pixel does not matter.
        assert_eq!(raster.sample(-9.99, 49.99), Some(0.0));
    }
}
