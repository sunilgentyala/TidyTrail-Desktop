use serde::Serialize;

/// An axis-aligned rectangle in the same coordinate space as the bounding
/// rect passed into [`squarify`] (typically CSS pixels).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One item to place, carrying whatever the caller needs to identify it
/// (an index into a `Node::children` vector, for example) alongside the
/// value the layout should size it by.
pub struct Item<T> {
    pub value: f64,
    pub data: T,
}

/// Lays out `items` inside `bounds` using the squarified treemap algorithm
/// (Bruls, Huizing, van Wijk 2000), the same approach TreeSize/WinDirStat
/// use: repeatedly fills the shortest side of the remaining space with a
/// row of items chosen to keep aspect ratios as close to square as
/// possible, which keeps small files from degenerating into unreadable
/// slivers.
///
/// Items with a `value <= 0` are dropped; `items` need not be pre-sorted.
pub fn squarify<T: Clone>(items: &[Item<T>], bounds: Rect) -> Vec<(T, Rect)> {
    let mut sorted: Vec<&Item<T>> = items.iter().filter(|i| i.value > 0.0).collect();
    sorted.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap());

    let mut out = Vec::with_capacity(sorted.len());
    if sorted.is_empty() || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return out;
    }

    let total: f64 = sorted.iter().map(|i| i.value).sum();
    if total <= 0.0 {
        return out;
    }
    // Scale values to the bounds' area so worst-ratio comparisons operate
    // in the same units as the rectangle sides being filled.
    let scale = (bounds.w * bounds.h) / total;

    let mut remaining = bounds;
    let mut row: Vec<f64> = Vec::new();
    let mut row_items: Vec<&Item<T>> = Vec::new();
    let mut idx = 0;

    while idx < sorted.len() {
        let item = sorted[idx];
        let value = item.value * scale;
        let side = remaining.w.min(remaining.h);

        if row.is_empty() || worst_ratio(&row, side) >= worst_ratio(&extended(&row, value), side) {
            row.push(value);
            row_items.push(item);
            idx += 1;
        } else {
            remaining = lay_out_row(&row, &row_items, remaining, &mut out);
            row.clear();
            row_items.clear();
        }
    }
    if !row.is_empty() {
        lay_out_row(&row, &row_items, remaining, &mut out);
    }

    out
}

fn extended(row: &[f64], value: f64) -> Vec<f64> {
    let mut r = row.to_vec();
    r.push(value);
    r
}

/// The worst (largest) width/height ratio any rectangle in `row` would get
/// if the row were laid out along a strip of the given `side` length.
/// Lower is more square; squarify greedily minimizes this per row.
fn worst_ratio(row: &[f64], side: f64) -> f64 {
    if row.is_empty() || side <= 0.0 {
        return f64::INFINITY;
    }
    let sum: f64 = row.iter().sum();
    let max = row.iter().cloned().fold(f64::MIN, f64::max);
    let min = row.iter().cloned().fold(f64::MAX, f64::min);
    let side_sq = side * side;
    let sum_sq = sum * sum;
    ((side_sq * max) / sum_sq).max(sum_sq / (side_sq * min))
}

fn lay_out_row<T: Clone>(
    row: &[f64],
    row_items: &[&Item<T>],
    space: Rect,
    out: &mut Vec<(T, Rect)>,
) -> Rect {
    let row_sum: f64 = row.iter().sum();
    if row_sum <= 0.0 {
        return space;
    }

    if space.w >= space.h {
        // Vertical strip on the left, one item stacked per row of the strip.
        let strip_w = row_sum / space.h;
        let mut y = space.y;
        for (&value, item) in row.iter().zip(row_items.iter()) {
            let h = value / strip_w;
            out.push((
                item.data.clone(),
                Rect {
                    x: space.x,
                    y,
                    w: strip_w,
                    h,
                },
            ));
            y += h;
        }
        Rect {
            x: space.x + strip_w,
            y: space.y,
            w: space.w - strip_w,
            h: space.h,
        }
    } else {
        // Horizontal strip along the top, one item per column.
        let strip_h = row_sum / space.w;
        let mut x = space.x;
        for (&value, item) in row.iter().zip(row_items.iter()) {
            let w = value / strip_h;
            out.push((
                item.data.clone(),
                Rect {
                    x,
                    y: space.y,
                    w,
                    h: strip_h,
                },
            ));
            x += w;
        }
        Rect {
            x: space.x,
            y: space.y + strip_h,
            w: space.w,
            h: space.h - strip_h,
        }
    }
}
