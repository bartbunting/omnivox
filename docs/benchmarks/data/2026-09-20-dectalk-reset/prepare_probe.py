from pathlib import Path
import shutil,subprocess,hashlib,json,difflib
repo=Path('/home/bart/src/omnivox');root=Path('/home/bart/src/emacsvox/.benchmarks/dectalk-latency-2026-09-20')
dest=root/'probe-source/windows-helpers';dest.mkdir(parents=True,exist_ok=False)
for name in subprocess.check_output(['git','ls-files','windows-helpers'],cwd=repo,text=True).splitlines():
 p=Path(name);out=dest/p.relative_to('windows-helpers');out.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(repo/p,out)
def replace(s,old,new,count=1):
 assert s.count(old)==count,(old[:100],s.count(old));return s.replace(old,new)
p=dest/'common/OmnivoxHelperHost.cs';s=p.read_text();original=s
s=replace(s,'    private static readonly object OutputLock = new object();','    [ThreadStatic] internal static string ProbeRequestId;\n    private static readonly object OutputLock = new object();')
s=replace(s,'            OmnivoxHelperLog.Event("native_synthesis_started", request);','            OmnivoxHelperLog.ProbeRequestId = synthesis.RequestId.ToString(CultureInfo.InvariantCulture);\n            OmnivoxHelperLog.Event("native_synthesis_started", request);')
p.write_text(s);diff=''.join(difflib.unified_diff(original.splitlines(True),s.splitlines(True),fromfile='a/windows-helpers/common/OmnivoxHelperHost.cs',tofile='b/windows-helpers/common/OmnivoxHelperHost.cs'))
p=dest/'dectalk/OmnivoxDectalkCapture.cs';s=p.read_text();original=s
s=replace(s,'using System.Collections.Generic;','using System.Collections.Generic;\nusing System.Diagnostics;')
fields='''    private long[] probe;
    private long[,] probeCallbacks;
    private int probeCallbackCount;
    private string probeRequestId;
    private static readonly string[] ProbeNames = {
        "entry", "locked", "reset_begin", "reset_end", "capture_ready",
        "rate_begin", "rate_end", "text_begin", "text_ready", "speak_begin",
        "speak_end", "sync_begin", "sync_end", "sink_begin", "sink_end"
    };
    private void ReportProbe()
    {
        if (probe == null) return;
        StringBuilder text = new StringBuilder("request_id=" + probeRequestId);
        for (int i = 0; i < probe.Length; ++i)
            text.Append(" " + ProbeNames[i] + "_us=" +
                (probe[i] == 0 ? -1 : (probe[i] - probe[0]) * 1000000L / Stopwatch.Frequency));
        text.Append(" callback_count=" + probeCallbackCount);
        for (int i = 0; i < Math.Min(probeCallbackCount, 16); ++i)
        {
            text.Append(" cb" + i + "_us=" + (probeCallbacks[i,0] - probe[0]) * 1000000L / Stopwatch.Frequency);
            text.Append(" cb" + i + "_frames=" + probeCallbacks[i,1]);
            text.Append(" cb" + i + "_peak=" + probeCallbacks[i,2]);
            text.Append(" cb" + i + "_markers=" + probeCallbacks[i,3]);
        }
        OmnivoxHelperLog.Event("dectalk_latency_probe", text.ToString());
    }

'''
s=replace(s,'    internal OmnivoxDectalkCapture(string dllPath)',fields+'    internal OmnivoxDectalkCapture(string dllPath)')
s=replace(s,'        lock (synthesisLock)\n        {','''        long probeEntry = Stopwatch.GetTimestamp();
        lock (synthesisLock)
        {
            probe = new long[ProbeNames.Length];
            probeCallbacks = new long[16,4];
            probeCallbackCount = 0;
            probeRequestId = OmnivoxHelperLog.ProbeRequestId;
            probe[0] = probeEntry;
            probe[1] = Stopwatch.GetTimestamp();''')
s=replace(s,'            lock (resetLock) BeginCapture(sink, volume);','            lock (resetLock) BeginCapture(sink, volume);\n            probe[4] = Stopwatch.GetTimestamp();')
s=replace(s,'                    Check(native.TextToSpeechSetRate(handle, (uint)rate), "TextToSpeechSetRate");','                    probe[5] = Stopwatch.GetTimestamp();\n                    Check(native.TextToSpeechSetRate(handle, (uint)rate), "TextToSpeechSetRate");\n                    probe[6] = Stopwatch.GetTimestamp();')
s=replace(s,'                string indexedText = BuildTextWithIndexes(text,','                probe[7] = Stopwatch.GetTimestamp();\n                string indexedText = BuildTextWithIndexes(text,')
s=replace(s,'                EmitProgressiveAnchorMarkers(leadingAnchorMarkers, 0);','                probe[8] = Stopwatch.GetTimestamp();\n                EmitProgressiveAnchorMarkers(leadingAnchorMarkers, 0);')
s=replace(s,'                Speak(prefix + indexedText);','                probe[9] = Stopwatch.GetTimestamp();\n                Speak(prefix + indexedText);\n                probe[10] = Stopwatch.GetTimestamp();')
s=replace(s,'                Check(native.TextToSpeechSync(handle), "TextToSpeechSync");','                probe[11] = Stopwatch.GetTimestamp();\n                Check(native.TextToSpeechSync(handle), "TextToSpeechSync");\n                probe[12] = Stopwatch.GetTimestamp();')
s=replace(s,'            finally\n            {\n                lock (resetLock)','            finally\n            {\n                ReportProbe();\n                lock (resetLock)')
s=replace(s,'        Check(native.TextToSpeechReset(handle, false),\n            "TextToSpeechReset");','        probe[2] = Stopwatch.GetTimestamp();\n        Check(native.TextToSpeechReset(handle, false),\n            "TextToSpeechReset");\n        probe[3] = Stopwatch.GetTimestamp();')
s=replace(s,'        IntPtr bufferPointer = new IntPtr(parameter2);','        long probeCallbackEntered = Stopwatch.GetTimestamp();\n        IntPtr bufferPointer = new IntPtr(parameter2);')
s=replace(s,'                        Marshal.Copy(buffer.Data, audio, 0, count);','''                        Marshal.Copy(buffer.Data, audio, 0, count);
                        int probeIndex = probeCallbackCount++;
                        if (probeIndex < 16)
                        {
                            int peak = 0;
                            for (int j = 0; j < count; j += 2)
                                peak = Math.Max(peak, Math.Abs((int)(short)(audio[j] | audio[j+1] << 8)));
                            probeCallbacks[probeIndex,0] = probeCallbackEntered;
                            probeCallbacks[probeIndex,1] = count / 2;
                            probeCallbacks[probeIndex,2] = peak;
                            probeCallbacks[probeIndex,3] = buffer.NumberOfIndexMarks + buffer.NumberOfPhonemeChanges;
                        }''')
s=replace(s,'                    sink.Audio(readyAudio, 0, readyAudio.Length);','''                    bool probeFirst = probe[13] == 0;
                    if (probeFirst) probe[13] = Stopwatch.GetTimestamp();
                    sink.Audio(readyAudio, 0, readyAudio.Length);
                    if (probeFirst) probe[14] = Stopwatch.GetTimestamp();''')
s=replace(s,'            sink.Audio(audio, 0, audio.Length);','''            bool probeFirst = probe[13] == 0;
            if (probeFirst) probe[13] = Stopwatch.GetTimestamp();
            sink.Audio(audio, 0, audio.Length);
            if (probeFirst) probe[14] = Stopwatch.GetTimestamp();''')
p.write_text(s);diff+=''.join(difflib.unified_diff(original.splitlines(True),s.splitlines(True),fromfile='a/windows-helpers/dectalk/OmnivoxDectalkCapture.cs',tofile='b/windows-helpers/dectalk/OmnivoxDectalkCapture.cs'))
(root/'probe.patch').write_text(diff)
manifest={'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip(),'files':{str(p.relative_to(dest)):hashlib.sha256(p.read_bytes()).hexdigest() for p in dest.rglob('*') if p.is_file()}}
(root/'probe-source.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(dest)
