//! Shared support for empirically calibrated speech-rate curves.

/// Interpolate a native engine control from normalized host-rate control points.
///
/// Points must be ordered by ascending host rate. Values below or above the
/// measured range saturate at the corresponding endpoint. A non-finite host
/// rate falls back to the normal host rate (`0.5`).
pub fn interpolate(rate: f32, points: &[(f32, f32)]) -> f32 {
    assert!(
        !points.is_empty(),
        "rate calibration requires control points"
    );
    debug_assert!(points.windows(2).all(|pair| pair[0].0 < pair[1].0));

    let rate = if rate.is_finite() { rate } else { 0.5 };
    if rate <= points[0].0 {
        return points[0].1;
    }
    for pair in points.windows(2) {
        let (lower_rate, lower_value) = pair[0];
        let (upper_rate, upper_value) = pair[1];
        if rate <= upper_rate {
            let position = (rate - lower_rate) / (upper_rate - lower_rate);
            return lower_value + (upper_value - lower_value) * position;
        }
    }
    points[points.len() - 1].1
}

#[cfg(test)]
mod tests {
    use super::interpolate;

    const POINTS: &[(f32, f32)] = &[(0.0, 10.0), (0.5, 20.0), (1.0, 40.0)];

    #[test]
    fn interpolates_each_segment() {
        assert_eq!(interpolate(0.0, POINTS), 10.0);
        assert_eq!(interpolate(0.25, POINTS), 15.0);
        assert_eq!(interpolate(0.5, POINTS), 20.0);
        assert_eq!(interpolate(0.75, POINTS), 30.0);
        assert_eq!(interpolate(1.0, POINTS), 40.0);
    }

    #[test]
    fn saturates_and_handles_non_finite_input() {
        assert_eq!(interpolate(-1.0, POINTS), 10.0);
        assert_eq!(interpolate(2.0, POINTS), 40.0);
        assert_eq!(interpolate(f32::NAN, POINTS), 20.0);
        assert_eq!(interpolate(f32::INFINITY, POINTS), 20.0);
    }
}
