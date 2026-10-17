//! A reader for `map.info`, the description of a map folder.
//!
//! Every folder under `media/maps` has a `map.info`. It is a list of `key=value` lines. They give
//! the map's title, the map folders whose cells it loads, and the center point of the spawn
//! selection map:
//!
//! ```text
//! title=See media/lua/shared/Translate/EN/Riverside, KY/title.txt
//! lots=Muldraugh, KY
//! zoomX=6300
//! zoomY=5668
//! zoomS=14.5
//! ```
//!
//! [`MapInfo::parse`] never fails. It skips lines it cannot use, as the game does.

/// The fields of a `map.info` that this crate reads.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MapInfo {
    /// The `title=` value as written.
    ///
    /// The maps that ship with the game put the path of a translation file here instead of a
    /// name.
    pub title: Option<String>,
    /// Every `lots=` value, in file order. These are the map folders whose cells this one loads.
    pub lots: Vec<String>,
    /// The center of the spawn selection map, as a world position from `zoomX` and `zoomY`.
    ///
    /// It is `None` unless both values are present and are numbers.
    pub zoom: Option<(f64, f64)>,
}

impl MapInfo {
    /// Reads a `map.info`.
    ///
    /// Invalid UTF-8 is replaced. Lines that are not `key=value`, or whose number does not
    /// parse, are skipped, as the game skips them.
    ///
    /// # Examples
    ///
    /// ```
    /// use libpzmap::sidecar::map_info::MapInfo;
    ///
    /// let info = MapInfo::parse(b"title=Riverside\nlots=Muldraugh, KY\nzoomX=6300\nzoomY=5668\n");
    /// assert_eq!(info.title.as_deref(), Some("Riverside"));
    /// assert_eq!(info.lots, ["Muldraugh, KY"]);
    /// assert_eq!(info.zoom, Some((6300.0, 5668.0)));
    /// ```
    pub fn parse(input: &[u8]) -> Self {
        let text = String::from_utf8_lossy(input);
        let mut info = MapInfo::default();
        let mut zoom_x = None;
        let mut zoom_y = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "title" if !value.is_empty() => info.title = Some(value.to_owned()),
                "lots" if !value.is_empty() => info.lots.push(value.to_owned()),
                "zoomX" => zoom_x = value.parse::<f64>().ok().filter(|v| v.is_finite()),
                "zoomY" => zoom_y = value.parse::<f64>().ok().filter(|v| v.is_finite()),
                _ => {}
            }
        }
        info.zoom = zoom_x.zip(zoom_y);
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_read_and_lots_repeat() {
        let info = MapInfo::parse(
            b"title=Riverside\r\nlots=Muldraugh, KY\nlots=Other\nfixed2x=true\nzoomX=6300\nzoomY=5668.5\nzoomS=14.5\nnot a pair\n",
        );
        assert_eq!(
            info,
            MapInfo {
                title: Some("Riverside".into()),
                lots: vec!["Muldraugh, KY".into(), "Other".into()],
                zoom: Some((6300.0, 5668.5)),
            }
        );
    }

    #[test]
    fn a_missing_or_broken_coordinate_means_no_zoom() {
        assert_eq!(MapInfo::parse(b"zoomX=1\n").zoom, None);
        assert_eq!(MapInfo::parse(b"zoomX=1\nzoomY=abc\n").zoom, None);
    }
}
