use omnivox_audio::{AudioBuffer, ProgressivePcmCanonicalizer};
use std::{env,fs};
fn save(path:&str, samples:&[f32]) {let bytes=samples.iter().flat_map(|s|s.to_le_bytes()).collect::<Vec<_>>();fs::write(path,bytes).unwrap();}
fn main(){let args=env::args().collect::<Vec<_>>();let bytes=fs::read(&args[1]).unwrap();let rate:u32=args[2].parse().unwrap();let channels:u16=args[3].parse().unwrap();let samples=bytes.chunks_exact(2).map(|b|i16::from_le_bytes([b[0],b[1]])).collect::<Vec<_>>();
 let floats=samples.iter().map(|s|*s as f32/32768.0).collect::<Vec<_>>();let buffered=AudioBuffer::try_from_interleaved_f32(floats,rate,channels).unwrap();save(&format!("{}.buffered.f32",args[4]),&buffered.samples);
 let mut converter=ProgressivePcmCanonicalizer::new(rate,channels).unwrap();let mut out=Vec::new();for part in samples.chunks(256*channels as usize){for b in converter.push_interleaved_i16(part).unwrap(){out.extend(b.samples);}}for b in converter.finish().unwrap(){out.extend(b.samples);}save(&format!("{}.progressive.f32",args[4]),&out);println!("source={} buffered={} progressive={}",samples.len()/channels as usize,buffered.frame_count(),out.len()/2);
}