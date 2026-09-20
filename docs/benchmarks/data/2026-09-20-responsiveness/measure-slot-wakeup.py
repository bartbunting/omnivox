"""Extract the actual admission and lease code and measure release-to-acquisition.

The native engine is replaced by an occupied slot released after 2 ms. This is
an isolated scheduling diagnostic, not speech or acoustic latency.
"""
import hashlib,json,subprocess
from pathlib import Path
root=Path(__file__).resolve().parent
repo=Path('/home/bart/src/omnivox')
source=repo/'omnivox-cli/src/engine_execution.rs'
baseline=subprocess.check_output(['git','show','3700d42:omnivox-cli/src/engine_execution.rs'],cwd=repo,text=True)
current=source.read_text()
shim='''
use std::sync::{mpsc,Arc,Mutex,Condvar};
use std::sync::atomic::{AtomicBool,AtomicU64,AtomicUsize,Ordering};
use std::thread;
use std::time::{Duration,Instant};
const MAX_ISOLATED_CALLS:usize=2;
const CANCELLATION_POLL_INTERVAL:Duration=Duration::from_millis(10);
const TRANSIENT_ENGINE_WAIT:Duration=Duration::from_millis(350);
macro_rules! warn { ($($args:tt)*) => {()} }
#[derive(Debug)] enum TtsError { NotAvailable }
struct SynthesisCancellationToken;
struct IsolatedTtsEngine { budget:Arc<IsolationBudget>, engine_active:Arc<AtomicBool> }
impl IsolatedTtsEngine {
 fn was_cancelled(&self,_:u64,_:u64,_:Option<&SynthesisCancellationToken>)->bool{false}
 fn cancellation_error(&self)->TtsError{TtsError::NotAvailable}
 METHOD
}
fn main(){
 let budget=Arc::new(IsolationBudget::new());
 let engine=IsolatedTtsEngine { budget:Arc::clone(&budget), engine_active:Arc::new(AtomicBool::new(false)) };
 for iteration in 0..120 {
  let lease=budget.try_acquire(&engine.engine_active).ok().unwrap();
  let (ready_tx,ready_rx)=mpsc::channel();
  let (released_tx,released_rx)=mpsc::channel();
  let worker=thread::spawn(move || {
   ready_tx.send(()).unwrap();
   thread::sleep(Duration::from_millis(2));
   let release=Instant::now();
   drop(lease);
   released_tx.send(release).unwrap();
  });
  ready_rx.recv().unwrap();
  let acquired=engine.acquire_for_current_generation("probe",0,0,None).unwrap();
  let now=Instant::now();
  let released=released_rx.recv().unwrap();
  if iteration>=20 {println!("{}",now.duration_since(released).as_nanos());}
  drop(acquired);worker.join().unwrap();
 }
}
'''
manifest={}
for name,s in [('polling',baseline),('notification',current)]:
 code=s[s.index('pub struct IsolationBudget'):s.index('/// Run one engine call away')]
 start=s.index('    fn acquire_for_current_generation(')
 end=s.index('\n}\n',start)
 method=s[start:end]
 text=shim.replace(' METHOD',method)+code
 p=root/(name+'-slot.rs');p.write_text(text)
 binary=root/(name+'-slot')
 subprocess.run(['rustc','--edition=2021','-O','-A','warnings',str(p),'-o',str(binary)],cwd=repo,check=True)
 manifest[name]={'input_source_sha256':hashlib.sha256(s.encode()).hexdigest(),'extracted_source_sha256':hashlib.sha256(text.encode()).hexdigest(),'runs':[]}
for block in range(3):
 for name in (['polling','notification'] if block%2==0 else ['notification','polling']):
  values=[int(v) for v in subprocess.check_output([str(root/(name+'-slot'))],text=True).splitlines()]
  manifest[name]['runs'].append(values)
(root/'slot-results.json').write_text(json.dumps(manifest,indent=2)+'\n')
import statistics
for name,m in manifest.items():
 values=sorted(v/1e6 for run in m['runs'] for v in run)
 print(name,'n',len(values),'median_ms',statistics.median(values),'p95_ms',values[int(.95*len(values))-1])
