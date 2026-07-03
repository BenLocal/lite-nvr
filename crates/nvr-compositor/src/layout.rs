//! The output canvas and the rectangle each source occupies on it. Free-form:
//! regions may be any size/position and overlap (picture-in-picture). A grid
//! helper builds evenly-tiled regions from a source list.

/// Round down to an even number (YUV420P chroma requires even geometry).
pub fn even(v: u32) -> u32 {
    v & !1
}

/// One source placed on the canvas.
#[derive(Clone, Debug)]
pub struct Region {
    pub source_id: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// The output canvas plus the regions composited onto it, back-to-front.
#[derive(Clone, Debug)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub regions: Vec<Region>,
}

impl Layout {
    pub fn new(width: u32, height: u32, regions: Vec<Region>) -> Self {
        Self {
            width: even(width),
            height: even(height),
            regions,
        }
    }

    /// An evenly-tiled grid covering the canvas, in `source_ids` order. Uses
    /// `ceil(sqrt(n))` columns (e.g. 2→1x2, 4→2x2, 5..9→3x3).
    pub fn grid(width: u32, height: u32, source_ids: &[String]) -> Self {
        let n = source_ids.len().max(1) as u32;
        let cols = (n as f64).sqrt().ceil() as u32;
        let rows = n.div_ceil(cols);
        let cw = even(width / cols);
        let ch = even(height / rows);
        let regions = source_ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let i = i as u32;
                Region {
                    source_id: id.clone(),
                    x: even((i % cols) * cw),
                    y: even((i / cols) * ch),
                    w: cw,
                    h: ch,
                }
            })
            .collect();
        Self::new(width, height, regions)
    }
}
