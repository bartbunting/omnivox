/* SPDX-License-Identifier: MIT
 * Test-only frontend substitute. Never staged by the prototype builder.
 */
#include <stdio.h>
#include <stdlib.h>
#ifdef _WIN32
#include <windows.h>
#include <tlhelp32.h>
static unsigned long parent_pid(void) {
    PROCESSENTRY32 entry;
    HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    unsigned long parent = 0;
    entry.dwSize = sizeof(entry);
    if (Process32First(snapshot, &entry)) do {
        if (entry.th32ProcessID == GetCurrentProcessId()) parent = entry.th32ParentProcessID;
    } while (Process32Next(snapshot, &entry));
    CloseHandle(snapshot);
    return parent;
}
#else
#include <unistd.h>
#endif

int main(int argc, char **argv) {
#ifdef _WIN32
    if (argc == 3) {
        HANDLE process = OpenProcess(SYNCHRONIZE | PROCESS_TERMINATE, 0, strtoul(argv[2], NULL, 10));
        if (!process) return GetLastError() == ERROR_INVALID_PARAMETER ? 0 : 2;
        if (argv[1][2] == 'k') TerminateProcess(process, 1);
        DWORD status = WaitForSingleObject(process, 3000);
        CloseHandle(process);
        return status == WAIT_OBJECT_0 ? 0 : 3;
    }
#else
    (void)argc; (void)argv;
#endif
    if (getchar() == 'H') {
        FILE *pid = fopen(getenv("OMNIVOX_MBROLA_TEST_PID_FILE"), "w");
        if (!pid) return 2;
#ifdef _WIN32
        fprintf(pid, "%lu %lu\n", (unsigned long)GetCurrentProcessId(), parent_pid());
        fclose(pid);
        Sleep(60000);
#else
        fprintf(pid, "%lu %lu\n", (unsigned long)getpid(), (unsigned long)getppid());
        fclose(pid);
        sleep(60);
#endif
    } else {
        fputs("h 70\n@ 24 0 94 100 99\nl 65\n@U 175 0 102 100 76\n_ 301\n_ 1\n", stdout);
    }
    return 0;
}
