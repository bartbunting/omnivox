use omnivox_audio::ProgressivePcmCanonicalizer;
fn main() {
 let mut c=ProgressivePcmCanonicalizer::new(11025,1).unwrap();
 for count in [512,482] {
  let windows=c.push_interleaved_f32(&vec![0.25;count]).unwrap();
  println!("native_chunk={} published={:?} total_canonical={}",count,windows.iter().map(|w|w.frame_count()).collect::<Vec<_>>(),c.output_frames());
 }
 let windows=c.finish().unwrap();
 println!("finish={:?} total_canonical={}",windows.iter().map(|w|w.frame_count()).collect::<Vec<_>>(),c.output_frames());
}
