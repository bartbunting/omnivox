param([Parameter(Mandatory=$true)][string]$Executable,
      [Parameter(Mandatory=$true)][string]$Data,
      [Parameter(Mandatory=$true)][string]$Root)
$ErrorActionPreference = 'Stop'
if (Test-Path $Root) { throw 'Evidence destination already exists' }
New-Item -ItemType Directory $Root | Out-Null
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using System.Runtime.InteropServices;
using System.IO.Pipes;
using System.Security.Principal;
public sealed class GuardProbePeer {
 public Process Process;
 public Task<string> Pending;
 public Task<string> Errors;
 public string Read(int milliseconds) {
  if(Pending==null) Pending=Process.StandardOutput.ReadLineAsync();
  if(!Pending.Wait(milliseconds)) throw new TimeoutException("Native reply timeout");
  string value=Pending.Result;Pending=null;
  if(value==null) throw new EndOfStreamException("Native control EOF");
  return value;
 }
 public void Send(string line){Process.StandardInput.WriteLine(line);Process.StandardInput.Flush();}
 public Task Fill(string line,int count) {
  return Task.Run(()=>{for(int i=0;i<count;i++)Send(line);});
 }
 [DllImport("ntdll.dll")] static extern int NtSuspendProcess(IntPtr handle);
 [DllImport("ntdll.dll")] static extern int NtResumeProcess(IntPtr handle);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool TerminateProcess(IntPtr handle,uint status);
 [DllImport("kernel32.dll",SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
 public static bool Ended(Process process) {return WaitForSingleObject(process.Handle,0)==0;}
 public static void Pause(Process process) {if(NtSuspendProcess(process.Handle)!=0)throw new Exception("Suspend failed");}
 public static void Resume(Process process) {if(!Ended(process)&&NtResumeProcess(process.Handle)!=0)throw new Exception("Resume failed");}
 public static void Kill(Process process) {if(!Ended(process)&&!TerminateProcess(process.Handle,1)){int error=Marshal.GetLastWin32Error();if(WaitForSingleObject(process.Handle,2000)!=0)throw new System.ComponentModel.Win32Exception(error,"Terminate retained fixture handle failed");}}
 public static bool Wait(Process process,int milliseconds) {return WaitForSingleObject(process.Handle,(uint)milliseconds)==0;}
}
public sealed class GuardProbePipe : IDisposable {
 public NamedPipeClientStream Pipe;
 public StreamReader Input;
 public StreamWriter Output;
 public GuardProbePipe(string name) {
  Pipe=new NamedPipeClientStream(".",name,PipeDirection.InOut,PipeOptions.Asynchronous,TokenImpersonationLevel.Identification);
  Pipe.Connect(5000);Input=new StreamReader(Pipe);Output=new StreamWriter(Pipe,new System.Text.UTF8Encoding(false));Output.AutoFlush=true;
 }
 public string Read(){var task=Input.ReadLineAsync();if(!task.Wait(10000))throw new TimeoutException("Raw status timeout");if(task.Result==null)throw new EndOfStreamException();return task.Result;}
 public void Send(string text){Output.WriteLine(text);}
 public Task Fill(string text){return Task.Run(()=>Output.Write(text));}
 public void Dispose(){Pipe.Dispose();}
}
'@
$utf8 = New-Object Text.UTF8Encoding($false)
$all = New-Object Collections.Generic.List[object]
$identities = New-Object Collections.Generic.List[object]
$records = New-Object Collections.Generic.List[object]
$permissions = New-Object Collections.Generic.List[object]
$raw = $null
function Save($path,$value) {[IO.File]::WriteAllText($path,($value|ConvertTo-Json -Depth 50),$utf8)}
function Fingerprints($folder) {
 $rows=@(Get-ChildItem (Join-Path $folder 'voices') -Recurse -File | Sort-Object FullName | ForEach-Object { $_.FullName+' '+(Get-FileHash -Algorithm SHA256 $_.FullName).Hash })
 return ($rows -join "`n")
}
function Start-Peer($arguments,$folder,$label) {
 $info=New-Object Diagnostics.ProcessStartInfo
 $info.FileName=$Executable;$info.Arguments=$arguments;$info.UseShellExecute=$false;$info.CreateNoWindow=$true
 $info.RedirectStandardInput=$true;$info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
 $keep=@('SystemRoot','WINDIR','COMSPEC','PATH','PATHEXT','TEMP','TMP','USERPROFILE','HOMEDRIVE','HOMEPATH','APPDATA','LOCALAPPDATA','PROGRAMDATA','PROGRAMFILES','PROGRAMFILES(X86)','PROCESSOR_ARCHITECTURE','OS','NUMBER_OF_PROCESSORS','SYSTEMDRIVE','USERNAME','USERDOMAIN')
 foreach($key in @($info.EnvironmentVariables.Keys)){if($key -notin $keep){$info.EnvironmentVariables.Remove($key)}}
 $info.EnvironmentVariables['OMNIVOX_CONFIG_DIR']=$folder
 $info.EnvironmentVariables['OMNIVOX_VOICE_ROOT']=(Join-Path $folder 'voices')
 $info.EnvironmentVariables['OMNIVOX_ENGINE']='flite';$info.EnvironmentVariables['OMNIVOX_AUDIO_OUTPUT']='null'
 $info.EnvironmentVariables['ESPEAK_NG_DATA']=$Data
 $p=New-Object GuardProbePeer;$p.Process=[Diagnostics.Process]::Start($info)
 $null=$p.Process.Handle # Retain actual creation identity before any fault.
 $p.Errors=$p.Process.StandardError.ReadToEndAsync()
 $all.Add(@{peer=$p;log=(Join-Path $folder "$label.stderr")})
 return $p
}
function Read-Json($p) {return ($p.Read(8000) -replace '^OMNIVOX-LOCAL ','')|ConvertFrom-Json}
function Send-Local($p,$command,$worker='') {
 $p.Send('OMNIVOX-LOCAL '+(@{command=$command;worker=$worker;request_id=1}|ConvertTo-Json -Compress))
 return Read-Json $p
}
function Guard($f,$command) {
 $f.sequence++
 $f.bridge.Send((@{command=$command;request_id=$f.sequence;guard=$f.endpoint.guard;worker=$f.endpoint.worker;token=$f.endpoint.token}|ConvertTo-Json -Compress))
 $r=Read-Json $f.bridge
 if($r.guard -ne $f.endpoint.guard -or $r.worker -ne $f.endpoint.worker -or $r.request_id -ne $f.sequence -or $r.error){throw "Invalid guard response: $($r|ConvertTo-Json -Compress)"}
 return $r
}
function Child($parent,$path) {
 $deadline=[DateTime]::UtcNow.AddSeconds(6)
 do {
  if([GuardProbePeer]::Ended($parent)){throw 'Fixture parent ended during native observation'}
  $observed=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($parent.Id)")
  Save (Join-Path $Root ("child-observation-"+$parent.Id+".json")) @($observed | Select-Object ProcessId,ParentProcessId,ExecutablePath,CreationDate)
  $rows=@($observed | Where-Object {$_.ExecutablePath.Replace('\\?\','') -eq $path.Replace('\\?\','')})
  if($rows.Count -eq 1){
   $row=$rows[0];$p=[Diagnostics.Process]::GetProcessById($row.ProcessId);$null=$p.Handle
   if($p.MainModule.FileName.Replace('\\?\','') -ne $path.Replace('\\?\','') -or $p.StartTime -lt $parent.StartTime -or [Math]::Abs(($p.StartTime-$row.CreationDate).TotalMilliseconds) -gt 2){$p.Dispose();throw 'Fixture child identity changed'}
   $identities.Add($p);return $p
  }
  Start-Sleep -Milliseconds 10
 }while([DateTime]::UtcNow -lt $deadline)
 throw ("Expected exact native fixture child absent: parent="+$parent.Id+" path="+$path)
}
function Fixture($folder,$label) {
 New-Item -ItemType Directory $folder | Out-Null
 $overrides=@{};foreach($id in @('winrt','macos','piper','rhvoice','rutts','tgspeechbox','eloquence','dectalk','mbrola')){$overrides[$id]=@{enabled=$false}}
 Save (Join-Path $folder 'config.json') @{schema=4;engine_overrides=$overrides}
 $p=Start-Peer '--voice-library-owner --guarded' $folder $label
 $owner=Send-Local $p 'describe'
 if(-not $owner.prepared -or $owner.startup_error){throw "Preparation failed: $($owner|ConvertTo-Json -Compress -Depth 8)"}
 $g=Child $p.Process $Executable
 $early=@(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($g.Id)")
 Save (Join-Path $folder 'guard-children-before-attach.json') @($early | Select-Object ProcessId,ParentProcessId,ExecutablePath,CreationDate)
 if(@($early | Where-Object {$_.ExecutablePath -and $_.ExecutablePath.Replace('\\?\','').StartsWith((Split-Path $Executable))}).Count){throw 'Native worker before client attachment'}
 $bridge=Start-Peer '--native-guard-control' $folder 'bridge'
 $f=@{owner=$p;guard=$g;bridge=$bridge;endpoint=$owner.guard;sequence=0;folder=$folder}
 $bridge.Send(($owner.guard|ConvertTo-Json -Compress));$null=Guard $f 'attach'
 $r=Send-Local $p 'start-guarded' $owner.worker
 if($r.type -ne 'started'){throw 'Worker not released'}
 $f.worker=Child $g $Executable
 Speech $p 100 'flite'
 $f.helper=Child $g (Join-Path (Split-Path $Executable) 'flite\omnivox-flite-helper.exe')
 return $f
}
function Speech($p,$id,$engine) {
 $json=@{protocol_version=3;generation=$id;dispatch_id=$id;delivery_policy='ordered';spans=@(@{id=1;text='Native recovery speech.'});actions=@()}|ConvertTo-Json -Compress -Depth 8
 $p.Send('emacsvox_timeline '+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($json)))
 $started=$false;$deadline=[DateTime]::UtcNow.AddSeconds(10)
 do{
  $line=$p.Read(8000)
  if($line.StartsWith('__EMACSVOX_MARKER__ ')){
   $marker=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($line.Split(' ')[1]))|ConvertFrom-Json
   if($marker.dispatch_id -eq $id -and $marker.type -eq 'utterance_started'){if($marker.engine_id -ne $engine){throw 'Wrong speech engine'};$started=$true}
  }
  if($line.StartsWith("__EMACSVOX_TRACKED__ $id ")){if(-not $started -or $line.Trim() -ne "__EMACSVOX_TRACKED__ $id completed"){throw "Speech failed $line"};return}
 }while([DateTime]::UtcNow -lt $deadline)
 throw 'Speech completion absent'
}
function Control($p,$request) {
 $p.Send('omnivox_control '+[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($request|ConvertTo-Json -Compress -Depth 8))))
 do {
  $line=$p.Read(8000)
  if($line.StartsWith('__OMNIVOX_CONTROL__ ')){
   $reply=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($line.Split(' ')[1]))|ConvertFrom-Json
   if($reply.request_id -eq $request.request_id){if($reply.type -eq 'error'){throw ($reply|ConvertTo-Json -Compress)};return $reply}
  }
 }while($true)
}
function Proof($f) {
 $timer=[Diagnostics.Stopwatch]::StartNew();$r=Guard $f 'retire'
 while(-not $r.tree_retired){if($timer.ElapsedMilliseconds -gt 6000){throw 'Tree proof timeout'};$r=Guard $f 'status'}
 if(-not [GuardProbePeer]::Ended($f.worker) -or -not [GuardProbePeer]::Ended($f.helper)){throw 'False tree proof'}
 $null=Guard $f 'retire-owner';$r=Guard $f 'status'
 while(-not $r.io_retired){if($timer.ElapsedMilliseconds -gt 6000){throw 'I/O proof timeout'};$r=Guard $f 'status'}
 return @{elapsed_ms=$timer.Elapsed.TotalMilliseconds;proof=$r}
}
try {
 $healthy=Fixture (Join-Path $Root 'healthy') 'notification'
 $noticeSequence=300
 foreach($case in @('owner-killed','owner-paused','blocked-input','blocked-output','helper-survives-worker','bridge-disconnected','helper-killed','held-status','guard-lost')) {
  $f=Fixture (Join-Path $Root $case) 'foreground';$row=@{case=$case};$fill=$null
  [IO.File]::WriteAllText((Join-Path $f.folder 'config.json'),'{broken',$utf8)
  [IO.File]::WriteAllText((Join-Path $f.folder 'voices\host.json'),'{broken',$utf8)
  $sessions=Join-Path $f.folder 'voices\sessions'
  $acl=Get-Acl $sessions;$permissions.Add(@{path=$sessions;acl=$acl})
  $denied=Get-Acl $sessions
  $rule=New-Object Security.AccessControl.FileSystemAccessRule([Security.Principal.WindowsIdentity]::GetCurrent().User,'Write','ContainerInherit,ObjectInherit','None','Deny')
  $denied.AddAccessRule($rule);Set-Acl $sessions $denied
  $before=Fingerprints $f.folder
  switch($case){
   'owner-killed' {[GuardProbePeer]::Kill($f.owner.Process)}
   'owner-paused' {[GuardProbePeer]::Pause($f.owner.Process)}
   'blocked-input' {[GuardProbePeer]::Pause($f.worker);$fill=$f.owner.Fill(('q {'+('x'*16000)+'}'),2000);Start-Sleep -Milliseconds 200;if($fill.IsCompleted){throw 'Speech input not blocked'}}
   'blocked-output' {$fill=$f.owner.Fill('OMNIVOX-LOCAL {"command":"describe","request_id":100}',5000);Start-Sleep -Milliseconds 200;if($fill.IsCompleted){throw 'Speech output not blocked'}}
   'helper-survives-worker' {[GuardProbePeer]::Pause($f.guard);[GuardProbePeer]::Pause($f.owner.Process);[GuardProbePeer]::Pause($f.helper);[GuardProbePeer]::Kill($f.worker);if(-not [GuardProbePeer]::Wait($f.worker,2000) -or [GuardProbePeer]::Ended($f.helper)){throw 'Surviving helper fixture absent'};[GuardProbePeer]::Resume($f.guard)}
   'bridge-disconnected' {
    $f.bridge.Process.StandardInput.Close();if(-not [GuardProbePeer]::Wait($f.bridge.Process,3000)){throw 'Bridge stdin close hung'}
    Speech $f.owner 101 'flite';$f.bridge=Start-Peer '--native-guard-control' $f.folder 'bridge-reconnect'
    $f.bridge.Send(($f.endpoint|ConvertTo-Json -Compress));$null=Guard $f 'attach';$row.speech_after_disconnect=$true
   }
   'helper-killed' {
    $null=Control $f.owner @{protocol_version=1;request_id=210;type='set_routing_policy';routing_policy_generation=1;preferred_engine_ids=@('flite');fallback_engine_ids=@('espeak');disabled_engine_ids=@()}
    foreach($attempt in 1..3){
     [GuardProbePeer]::Kill($f.helper)
     if(-not [GuardProbePeer]::Wait($f.helper,2000)){throw 'Helper fixture did not exit'}
     Speech $f.owner (110+2*$attempt) 'espeak'
     $null=Control $f.owner @{protocol_version=1;request_id=(210+$attempt);type='request_engine_recovery_probe';engine_id='flite'}
     Speech $f.owner (111+2*$attempt) 'flite'
     $f.helper=Child $f.guard (Join-Path (Split-Path $Executable) 'flite\omnivox-flite-helper.exe')
     if([GuardProbePeer]::Ended($f.worker)){throw 'Helper recovery replaced the worker'}
    }
    $row.helper_restarts=3
   }
   'held-status' {
    $f.bridge.Process.StandardInput.Close();if(-not [GuardProbePeer]::Wait($f.bridge.Process,3000)){throw 'Bridge did not close'}
    $raw=New-Object GuardProbePipe($f.endpoint.address.Substring(9))
    $raw.Send((@{command='attach';request_id=10000;guard=$f.endpoint.guard;worker=$f.endpoint.worker;token=$f.endpoint.token}|ConvertTo-Json -Compress))
    $attached=$raw.Read()|ConvertFrom-Json;if($attached.error){throw 'Raw attach rejected'}
    $batch=New-Object Text.StringBuilder
    foreach($n in 10001..11000){$null=$batch.AppendLine((@{command='status';request_id=$n;guard=$f.endpoint.guard;worker=$f.endpoint.worker}|ConvertTo-Json -Compress))}
    $null=$batch.AppendLine((@{command='status';request_id=12000;guard=$f.endpoint.guard;worker=$f.endpoint.worker}|ConvertTo-Json -Compress))
    $held=$raw.Fill($batch.ToString());Start-Sleep -Milliseconds 250
    if($held.IsCompleted){throw 'Raw status path not saturated'}
    $timer=[Diagnostics.Stopwatch]::StartNew();[GuardProbePeer]::Kill($f.owner.Process)
    if(-not [GuardProbePeer]::Wait($f.worker,2000) -or -not [GuardProbePeer]::Wait($f.helper,2000)){throw 'Status pressure blocked native tree retirement'}
    $row.native_exit_while_status_held_ms=$timer.Elapsed.TotalMilliseconds
    do{$last=$raw.Read()|ConvertFrom-Json;if($last.guard -ne $f.endpoint.guard -or $last.worker -ne $f.endpoint.worker){throw 'Foreign held response'}}while($last.request_id -ne 12000)
    if(-not $last.tree_retired -or -not $last.io_retired){throw 'Held reply lacked real cleanup'}
    if(-not $held.Wait(5000)){throw 'Status sender did not drain'};$raw.Dispose();$raw=$null
    $row.retirement=@{elapsed_ms=$timer.Elapsed.TotalMilliseconds;proof=$last}
   }
   'guard-lost' {[GuardProbePeer]::Kill($f.guard)}
  }
  if($case -eq 'guard-lost'){
   $failed=$false;try{$null=Guard $f 'status'}catch{$failed=$true}
   if(-not $failed){throw 'Dead guard returned proof'};$row.replacement_blocked=$true
  }else{
   if(-not $row.retirement){$row.retirement=Proof $f}
   $candidate=Start-Peer ('--emergency-owner --audio-output null --espeak-data "'+$Data+'"') $f.folder 'minimal'
   Speech $candidate 201 'espeak';$candidate.Process.StandardInput.Close();if(-not [GuardProbePeer]::Wait($candidate.Process,6000)){throw 'Minimal owner did not retire'}
   $row.minimal_speech=$true
  }
  if((Fingerprints $f.folder) -ne $before){throw 'Recovery changed management or asset state'}
  $row.claims_unchanged=$true
  $noticeSequence++
  Speech $healthy.owner $noticeSequence 'flite';$row.healthy_notification=$true
  if($fill){try{$null=$fill.Wait(3000)}catch{};if(-not $fill.IsCompleted){throw 'Fixture write survived owner retirement'}}
  $row.passed=$true;$records.Add($row);Save (Join-Path $Root 'result.json') $records
  Write-Output ("PASS "+$case)
 }
}catch{
 [IO.File]::WriteAllText((Join-Path $Root "failure.txt"),($_ | Out-String),$utf8)
 throw
}finally{
 if($raw){$raw.Dispose()}
 foreach($entry in $permissions){Set-Acl $entry.path $entry.acl}
 foreach($p in $identities){[GuardProbePeer]::Kill($p)}
 foreach($entry in $all){[GuardProbePeer]::Kill($entry.peer.Process)}
 foreach($p in $identities){if(-not [GuardProbePeer]::Wait($p,6000)){throw 'Native fixture survived cleanup'}}
 foreach($entry in $all){
  if(-not [GuardProbePeer]::Wait($entry.peer.Process,6000)){throw 'Fixture owner survived cleanup'}
  if($entry.peer.Errors.Wait(6000)){[IO.File]::WriteAllText($entry.log,$entry.peer.Errors.Result,$utf8)}
 }
 Save (Join-Path $Root 'result.json') $records
}
