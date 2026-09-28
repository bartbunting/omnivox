// Test-only process-tree loopback recorder. Copyright 2026 Omnivox contributors.
// SPDX-License-Identifier: MIT
// API contract: Microsoft's ApplicationLoopback sample (see evidence report).
#define _WIN32_WINNT 0x0A00
#include <windows.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <cstdio>
#include <vector>
#include <string>
#include <stdexcept>

void check(HRESULT hr, const char* name) {
    if (FAILED(hr)) { fprintf(stderr, "%s failed: 0x%08lx\n", name, (unsigned long)hr); throw std::runtime_error(name); }
}
struct Completion : IActivateAudioInterfaceCompletionHandler {
    LONG refs=1; HANDLE done=CreateEventW(nullptr,FALSE,FALSE,nullptr);
    HRESULT result=E_PENDING; IAudioClient* client=nullptr; IUnknown* marshaler=nullptr;
    Completion() { check(CoCreateFreeThreadedMarshaler(this,&marshaler),"marshaler"); }
    ~Completion(){ if(marshaler)marshaler->Release(); CloseHandle(done); }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id,void** out) override {
        if(id==__uuidof(IUnknown)||id==IID_IAgileObject||id==__uuidof(IActivateAudioInterfaceCompletionHandler)) {
            *out=static_cast<IActivateAudioInterfaceCompletionHandler*>(this); AddRef(); return S_OK;
        }
        if(id==IID_IMarshal)return marshaler->QueryInterface(id,out);
        *out=nullptr;return E_NOINTERFACE;
    }
    ULONG STDMETHODCALLTYPE AddRef() override {return InterlockedIncrement(&refs);}
    ULONG STDMETHODCALLTYPE Release() override {LONG n=InterlockedDecrement(&refs);if(!n)delete this;return n;}
    HRESULT STDMETHODCALLTYPE ActivateCompleted(IActivateAudioInterfaceAsyncOperation* op) override {
        IUnknown* obj=nullptr;HRESULT hr=op->GetActivateResult(&result,&obj);
        if(FAILED(hr))result=hr;
        if(SUCCEEDED(result))result=obj->QueryInterface(__uuidof(IAudioClient),(void**)&client);
        if(obj)obj->Release();SetEvent(done);return S_OK;
    }
};
int wmain(int argc,wchar_t** argv) {
    if(argc<5){fprintf(stderr,"capture OUT.f32 SECONDS PROGRAM [ARGS...]\n");return 2;}
    try {
        check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"CoInitializeEx");
        auto completion=new Completion();
        struct Params {int type; DWORD pid; int mode;} params{1,GetCurrentProcessId(),0};
        PROPVARIANT pv{};pv.vt=VT_BLOB;pv.blob.cbSize=sizeof(params);pv.blob.pBlobData=(BYTE*)&params;
        using Activate=HRESULT(WINAPI*)(LPCWSTR,REFIID,PROPVARIANT*,IActivateAudioInterfaceCompletionHandler*,IActivateAudioInterfaceAsyncOperation**);
        auto activate=(Activate)GetProcAddress(LoadLibraryW(L"Mmdevapi.dll"),"ActivateAudioInterfaceAsync");
        if(!activate)throw std::runtime_error("activation API missing");
        IActivateAudioInterfaceAsyncOperation* operation=nullptr;
        check(activate(L"VAD\\Process_Loopback",__uuidof(IAudioClient),&pv,completion,&operation),"activate");
        if(WaitForSingleObject(completion->done,10000)!=WAIT_OBJECT_0)throw std::runtime_error("activation timeout");
        check(completion->result,"activation result");
        auto client=completion->client;
        WAVEFORMATEX format{};format.wFormatTag=WAVE_FORMAT_IEEE_FLOAT;format.nChannels=2;
        format.nSamplesPerSec=44100;format.wBitsPerSample=32;format.nBlockAlign=8;format.nAvgBytesPerSec=352800;
        check(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_EVENTCALLBACK|AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,0,0,&format,nullptr),"Initialize");
        HANDLE event=CreateEventW(nullptr,FALSE,FALSE,nullptr);
        check(client->SetEventHandle(event),"SetEventHandle");
        IAudioCaptureClient* capture=nullptr;check(client->GetService(__uuidof(IAudioCaptureClient),(void**)&capture),"GetService");
        FILE* output=_wfopen(argv[1],L"wb");
        std::wstring logname=std::wstring(argv[1])+L".packets.csv";
        FILE* log=_wfopen(logname.c_str(),L"wb");
        if(!output||!log)throw std::runtime_error("open output");
        fprintf(log,"file_frame,device_frame,qpc_100ns,frames,flags\n");
        check(client->Start(),"Start");
        // Capture is running before any child output stream exists.
        std::wstring command;
        for(int i=3;i<argc;++i){if(i>3)command+=L" ";command+=L"\"";command+=argv[i];command+=L"\"";}
        STARTUPINFOW si{};si.cb=sizeof(si);si.dwFlags=STARTF_USESTDHANDLES;
        si.hStdInput=GetStdHandle(STD_INPUT_HANDLE);si.hStdOutput=GetStdHandle(STD_OUTPUT_HANDLE);si.hStdError=GetStdHandle(STD_ERROR_HANDLE);
        PROCESS_INFORMATION pi{};
        if(!CreateProcessW(nullptr,&command[0],nullptr,nullptr,TRUE,0,nullptr,nullptr,&si,&pi))throw std::runtime_error("CreateProcess");
        HANDLE job=CreateJobObjectW(nullptr,nullptr); JOBOBJECT_EXTENDED_LIMIT_INFORMATION limit{};
        limit.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if(!SetInformationJobObject(job,JobObjectExtendedLimitInformation,&limit,sizeof(limit))||!AssignProcessToJobObject(job,pi.hProcess))throw std::runtime_error("job setup");
        fprintf(stderr,"CAPTURE_READY recorder=%lu child=%lu rate=44100 channels=2 float32\n",GetCurrentProcessId(),pi.dwProcessId);fflush(stderr);
        ULONGLONG end=GetTickCount64()+(ULONGLONG)(_wtof(argv[2])*1000);unsigned long long offset=0;
        while(GetTickCount64()<end) {
            WaitForSingleObject(event,50);UINT32 available=0;
            check(capture->GetNextPacketSize(&available),"GetNextPacketSize");
            while(available) {
                BYTE* data=nullptr;UINT32 frames=0;DWORD flags=0;UINT64 position=0,qpc=0;
                check(capture->GetBuffer(&data,&frames,&flags,&position,&qpc),"GetBuffer");
                fprintf(log,"%llu,%llu,%llu,%u,%lu\n",offset,(unsigned long long)position,(unsigned long long)qpc,frames,flags);
                if(flags&AUDCLNT_BUFFERFLAGS_SILENT){std::vector<float> zeros(frames*2,0);fwrite(zeros.data(),8,frames,output);}
                else fwrite(data,8,frames,output);
                offset+=frames;check(capture->ReleaseBuffer(frames),"ReleaseBuffer");
                check(capture->GetNextPacketSize(&available),"GetNextPacketSize");
            }
        }
        check(client->Stop(),"Stop");fclose(output);fclose(log);
        DWORD exit=STILL_ACTIVE;GetExitCodeProcess(pi.hProcess,&exit);
        fprintf(stderr,"CAPTURE_DONE frames=%llu child_exit=%lu\n",offset,exit);
        CloseHandle(job);CloseHandle(pi.hThread);CloseHandle(pi.hProcess);
        capture->Release();client->Release();operation->Release();completion->Release();CloseHandle(event);CoUninitialize();
        return 0;
    }catch(const std::exception& e){fprintf(stderr,"ERROR %s\n",e.what());return 1;}
}
