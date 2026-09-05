// Test-only raw stdio/TCP relay for WSL NAT -> Windows loopback acceptance.
// Copyright (c) 2026 Omnivox contributors. SPDX-License-Identifier: MIT
using System;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Threading;

class RemoteTestBridge {
    static int Main(string[] args) {
        try {
            using (var client = new TcpClient()) {
                client.Connect(IPAddress.Loopback, int.Parse(args[0]));
                client.NoDelay = true;
                var network = client.GetStream();
                var input = new Thread(() => {
                    try {
                        Console.OpenStandardInput().CopyTo(network);
                        client.Client.Shutdown(SocketShutdown.Send);
                    } catch (IOException) {} catch (SocketException) {} catch (ObjectDisposedException) {}
                });
                input.IsBackground = true;
                input.Start();
                network.CopyTo(Console.OpenStandardOutput());
            }
            return 0;
        } catch (Exception error) {
            Console.Error.WriteLine(error.Message);
            return 1;
        }
    }
}
