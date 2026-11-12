from pathlib import Path
import hashlib,json,shutil,datetime
stage=Path('/mnt/c/Users/bart/AppData/Local/Emacsvox/NativeLightweight/native-early-20261113')
p=stage/'startup-timing-20261113'
maintained=Path('/home/bart/src/workstation-config/windows/emacs')
bin=Path('/mnt/c/Users/bart/bin')
stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
backup=stage/'backups'/('startup-'+stamp)
backup.mkdir()
items=[]
def install(destination,content,label):
 old=destination.read_bytes()
 (backup/label).write_bytes(old)
 temporary=destination.with_name(destination.name+'.startup-new')
 temporary.write_bytes(content)
 temporary.replace(destination)
 items.append({'destination':str(destination),'backup':str(backup/label),'before_sha256':hashlib.sha256(old).hexdigest(),'after_sha256':hashlib.sha256(content).hexdigest()})
# Bootstrap is explicitly loaded as source on each launch.
install(bin/'evox-windows-init.el',(maintained/'evox-windows-init.el').read_bytes(),'evox-windows-init.el')
install(stage/'evox-windows.ps1',(p/'final-launcher.ps1').read_bytes(),'desktop-evox-windows.ps1')
s=(bin/'evox-windows.ps1').read_text()
s=s.replace('$selected = Get-EmacsvoxNativeEmacs $config.Emacs', '''$command = Get-Command $config.Emacs -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $command) { throw "Selected Emacs does not exist: $($config.Emacs)" }
$selected = @{ Program = $command.Source }''')
a=s.index('Invoke-EmacsvoxNative $selected.Program @(\'-Q\', \'--batch\'')
b=s.index('$arguments = @',a)
block=s[a:b]
s=s[:a]+'''# Deployment and explicit checks validate the complete build. Ordinary startup
# opens only the personal Emacs; its bootstrap checks the running Emacs version.
if ($Check) {
    $selected = Get-EmacsvoxNativeEmacs $config.Emacs
'''+''.join('    '+line+'\n' for line in block.splitlines())+'}\n'+s[b:]
install(bin/'evox-windows.ps1',s.encode(),'bin-evox-windows.ps1')
record={'timestamp_utc':stamp,'changed':items,'runtime_sha256':hashlib.sha256((stage/'runtime/omnivox.exe').read_bytes()).hexdigest(),'launcher_sha256':hashlib.sha256((stage/'evox-windows-launcher.exe').read_bytes()).hexdigest(),'client_commit':'0acf2deae811a920b49db325eaaa3f9b31a7e8ce'}
(stage/('startup-update-'+stamp+'.json')).write_text(json.dumps(record,indent=2)+'\n')
(p/'deployment.json').write_text(json.dumps(record,indent=2)+'\n')
shutil.copy2(maintained/'evox-windows.ps1',p/'maintained-launcher.ps1')
print(json.dumps(record,indent=2))
