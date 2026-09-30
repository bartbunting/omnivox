// Test-only native Windows counterpart of framework-helper.py.
// Copyright (c) 2026 Omnivox contributors. SPDX-License-Identifier: MIT
using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

class FrameworkHelper {
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();

    static void Reply(object requestId, string kind, params object[] fields) {
        var reply = new Dictionary<string, object> {
            {"protocol_version", 5}, {"request_id", requestId}, {"type", kind}
        };
        for (int i = 0; i < fields.Length; i += 2)
            reply[(string)fields[i]] = fields[i + 1];
        Console.WriteLine(Json.Serialize(reply));
        Console.Out.Flush();
    }

    static void Main(string[] args) {
        // The same test executable can model a worker that never opens its
        // private startup pipe. Ordinary helper launches always have this flag.
        if (Array.IndexOf(args, "--descriptor") < 0) {
            Thread.Sleep(300000);
            return;
        }
        Console.InputEncoding = new UTF8Encoding(false);
        Console.OutputEncoding = new UTF8Encoding(false);
        var options = new Dictionary<string, string>();
        for (int i = 0; i < args.Length; i++) {
            if (args[i] == "--hang") { Thread.Sleep(300000); return; }
            string name = args[i];
            options[name] = args[++i];
        }
        File.AppendAllText(options["--record"], Json.Serialize(args) + "\n");
        if (options.ContainsKey("--record-environment")) {
            File.AppendAllText(options["--record-environment"], Json.Serialize(
                new Dictionary<string, object> {
                    {"pid", System.Diagnostics.Process.GetCurrentProcess().Id},
                    {"value", Environment.GetEnvironmentVariable("OMNIVOX_FIXTURE_PRIVATE")}
                }) + "\n");
        }
        var descriptor = Json.DeserializeObject(File.ReadAllText(options["--descriptor"]))
            as Dictionary<string, object>;
        string line;
        while ((line = Console.ReadLine()) != null) {
            var request = Json.DeserializeObject(line) as Dictionary<string, object>;
            object id = request["request_id"];
            switch ((string)request["type"]) {
            case "hello":
                Reply(id, "hello", "selected_protocol_version", 5,
                      "helper_name", "Framework fixture", "helper_version", "1");
                break;
            case "describe": Reply(id, "descriptor", "descriptor", descriptor); break;
            case "synthesize":
                var settings = request["settings"] as Dictionary<string, object>;
                object selectedVoice;
                if (!settings.TryGetValue("voice_id", out selectedVoice) || selectedVoice == null)
                    selectedVoice = descriptor["default_voice_id"];
                if (options.ContainsKey("--record-synthesis"))
                    File.AppendAllText(options["--record-synthesis"], Json.Serialize(request) + "\n");
                var pcm = File.ReadAllBytes(options["--pcm"]);
                Reply(id, "synthesis_started", "format", new Dictionary<string, object> {
                    {"sample_rate", Int32.Parse(options["--sample-rate"])}, {"channels", Int32.Parse(options["--channels"])}, {"sample_format", "pcm_s16_le"}
                }, "actual_voice_id", selectedVoice);
                int block = Int32.Parse(options["--chunk-frames"]) * 2 * Int32.Parse(options["--channels"]);
                for (int offset=0, sequence=0;offset<pcm.Length;offset+=block,sequence++) {
                    var part=new byte[Math.Min(block,pcm.Length-offset)];
                    Array.Copy(pcm,offset,part,0,part.Length);
                    Reply(id,"audio_chunk","chunk",new Dictionary<string,object> {
                        {"sequence",sequence},{"data_base64",Convert.ToBase64String(part)}
                    });
                    if (sequence == 0) {
                        int delay=Int32.Parse(options["--chunk-delay-ms"]);
                        var clock=System.Diagnostics.Stopwatch.StartNew();
                        if(delay>0)Thread.Sleep(delay);
                        Console.Error.WriteLine("FIRST_GAP_US="+(clock.ElapsedTicks*1000000/System.Diagnostics.Stopwatch.Frequency));
                    }
                }
                Reply(id, "synthesis_completed", "frame_count", pcm.Length / (2 * Int32.Parse(options["--channels"])));
                break;
            case "cancel": Reply(id, "cancel_accepted", "target_request_id", request["target_request_id"]); break;
            case "ping": Reply(id, "pong"); break;
            case "shutdown": Reply(id, "shutting_down"); return;
            }
        }
    }
}
