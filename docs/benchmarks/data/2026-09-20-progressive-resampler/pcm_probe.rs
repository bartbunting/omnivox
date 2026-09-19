use omnivox_audio::ProgressivePcmCanonicalizer;
use std::{hint::black_box, time::Instant};
fn main() {
    for rate in [11_025, 22_050, 44_100] {
        for iteration in 0..12 {
            let start = Instant::now();
            let converter = ProgressivePcmCanonicalizer::new(black_box(rate), 1).unwrap();
            let elapsed = start.elapsed();
            black_box(converter);
            println!("rate={rate} iteration={iteration} constructor_us={}", elapsed.as_micros());
        }
    }
}
