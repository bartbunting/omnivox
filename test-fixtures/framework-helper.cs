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
            {"protocol_version", 1}, {"request_id", requestId}, {"type", kind}
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
                Reply(id, "hello", "selected_protocol_version", 1,
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
                var pcm = new byte[4400];
                for (int frame = 0; frame < 2200; frame++) {
                    short sample = (short)(8192 * Math.Sin(2 * Math.PI * 440 * frame / 22050));
                    pcm[2 * frame] = (byte)sample;
                    pcm[2 * frame + 1] = (byte)(sample >> 8);
                }
                Reply(id, "synthesis_started", "format", new Dictionary<string, object> {
                    {"sample_rate", 22050}, {"channels", 1}, {"sample_format", "pcm_s16_le"}
                }, "actual_voice_id", selectedVoice);
                Reply(id, "audio_chunk", "chunk", new Dictionary<string, object> {
                    {"sequence", 0}, {"data_base64", Convert.ToBase64String(pcm)}
                });
                Reply(id, "synthesis_completed", "frame_count", 2200);
                break;
            case "cancel": Reply(id, "cancel_accepted", "target_request_id", request["target_request_id"]); break;
            case "ping": Reply(id, "pong"); break;
            case "shutdown": Reply(id, "shutting_down"); return;
            }
        }
    }
}
