//! Charts drawn with Unicode block elements: every cell holds one of nine
//! fill levels (`▁` to `█`), so a chart `h` rows tall shows `8h` steps.
//! They're plain strings built in one pass over the data, with no layout
//! engine behind them, so a frame costs microseconds.

use crate::term::{Color, Paint};

/// Fill levels from empty to full, in eighths.
const LEVELS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// One row: each value as a cell 0–8 eighths high against `max`. A value
/// above zero always shows at least the lowest block, so a trickle of
/// traffic isn't drawn as nothing.
pub fn sparkline(values: &[Option<f64>], max: f64) -> String {
    values
        .iter()
        .map(|v| match v {
            None => ' ',
            Some(v) => LEVELS[eighths(*v, max, 8).max(usize::from(*v > 0.0 && max > 0.0))],
        })
        .collect()
}

/// Column chart `height` rows tall, top row first.
pub fn columns(values: &[Option<f64>], max: f64, height: usize) -> Vec<String> {
    let height = height.max(1);
    let filled: Vec<usize> = values
        .iter()
        .map(|v| match v {
            None => 0,
            Some(v) => eighths(*v, max, height * 8).max(usize::from(*v > 0.0 && max > 0.0)),
        })
        .collect();
    (0..height)
        .rev()
        .map(|row| {
            filled
                .iter()
                .map(|&e| LEVELS[e.saturating_sub(row * 8).min(8)])
                .collect()
        })
        .collect()
}

/// `v / max` in `steps` (rounded), clamped.
fn eighths(v: f64, max: f64, steps: usize) -> usize {
    // NaN, zero and negative values all draw nothing.
    if max.is_nan() || v.is_nan() || max <= 0.0 || v <= 0.0 {
        return 0;
    }
    ((v / max).min(1.0) * steps as f64).round() as usize
}

/// `values` squeezed or padded into `n` slots, newest on the right: each
/// slot is the busiest value it covers. A history that hasn't filled
/// `capacity` yet leaves empty slots on the left, so it fills in from the
/// right as the dashboard's graph does.
pub fn resample(values: &[f64], capacity: usize, n: usize) -> Vec<Option<f64>> {
    let n = n.max(1);
    let capacity = capacity.max(values.len()).max(1);
    let per = capacity.div_ceil(n).max(1);
    let missing = (n * per).saturating_sub(values.len());
    (0..n)
        .map(|i| {
            let start = (i * per).saturating_sub(missing);
            let end = ((i + 1) * per).saturating_sub(missing).min(values.len());
            (start < end).then(|| values[start..end].iter().copied().fold(0.0, f64::max))
        })
        .collect()
}

/// A horizontal bar `width` cells wide, filled to `fraction`, in half-cell
/// steps: `━━━━━╸────`.
pub fn bar(p: &Paint, fraction: f64, width: usize, color: Color) -> String {
    let width = width.max(1);
    let halves = (fraction.clamp(0.0, 1.0) * (width * 2) as f64).round() as usize;
    let full = halves / 2;
    let half = halves % 2 == 1;
    let mut fill = "━".repeat(full);
    if half {
        fill.push('╸');
    }
    let rest = width - full - usize::from(half);
    format!("{}{}", p.paint(color, fill), p.dim("─".repeat(rest)))
}

/// A bar for a value inside a range, such as a fan between its minimum and
/// maximum speed.
pub fn range_bar(p: &Paint, value: f32, min: f32, max: f32, width: usize, color: Color) -> String {
    let span = (max - min).max(1.0);
    bar(p, ((value - min) / span) as f64, width, color)
}

/// A braille spinner frame, by tick.
pub fn spinner(tick: u64) -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    FRAMES[(tick % FRAMES.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_scales_to_the_peak() {
        let v = [Some(0.0), Some(1.0), Some(4.0), Some(8.0), None];
        assert_eq!(sparkline(&v, 8.0), " ▁▄█ ");
        // A trickle still shows.
        assert_eq!(sparkline(&[Some(0.01)], 8.0), "▁");
        assert_eq!(sparkline(&[Some(5.0)], 0.0), " ");
    }

    #[test]
    fn columns_stack_eighths_bottom_up() {
        let rows = columns(&[Some(16.0), Some(12.0), Some(2.0), None], 16.0, 2);
        assert_eq!(rows, ["█▄  ", "██▂ "]);
    }

    #[test]
    fn resample_fills_from_the_right_and_keeps_peaks() {
        let v = [1.0, 5.0, 2.0, 3.0];
        assert_eq!(resample(&v, 4, 2), [Some(5.0), Some(3.0)]);
        assert_eq!(resample(&v, 8, 4), [None, None, Some(5.0), Some(3.0)]);
        assert_eq!(resample(&v, 4, 8).iter().filter(|s| s.is_some()).count(), 4);
        assert!(resample(&[], 4, 3).iter().all(Option::is_none));
    }

    #[test]
    fn bars_fill_in_half_cells() {
        let p = Paint { on: false };
        assert_eq!(bar(&p, 0.0, 4, Color::Ok), "────");
        assert_eq!(bar(&p, 0.5, 4, Color::Ok), "━━──");
        assert_eq!(bar(&p, 0.625, 4, Color::Ok), "━━╸─");
        assert_eq!(bar(&p, 1.0, 4, Color::Ok), "━━━━");
        assert_eq!(bar(&p, 7.0, 2, Color::Ok), "━━");
        assert_eq!(range_bar(&p, 3000.0, 1000.0, 5000.0, 4, Color::Ok), "━━──");
    }
}
