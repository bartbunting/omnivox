#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
/* Test-only fault in the actual owner; guard, worker and helper are untouched. */
static int mode;
__attribute__((constructor)) static void configure(void) {
 char buf[4096]; int fd=open("/proc/self/cmdline",O_RDONLY);
 ssize_t n=read(fd,buf,sizeof(buf)); close(fd);
 int owner=0;
 for(ssize_t i=0;i<n;) {size_t len=strnlen(buf+i,n-i); if(!strcmp(buf+i,"--emergency-owner"))owner=1;i+=len+1;}
 const char *fault=getenv("OMNIVOX_GUARD_TEST_FAULT");
 if(owner && fault) mode=!strcmp(fault,"length")?1:!strcmp(fault,"body")?2:!strcmp(fault,"start")?3:0;
}
ssize_t write(int fd,const void *bytes,size_t size) {
 ssize_t (*actual)(int,const void*,size_t)=dlsym(RTLD_NEXT,"write");
 int cut=0;
 const unsigned char *b=bytes;
 if(fd>2 && mode==1 && size==4 && b[0]==0 && (b[1]||b[2]||b[3]))cut=2;
 if(fd>2 && mode==2 && size>32 && !memcmp(bytes,"{\"worker\":",10))cut=31;
 if(fd>2 && mode==3 && size==7 && !memcmp(bytes,"START1\n",7))cut=3;
 if(cut) {mode=0;ssize_t result=actual(fd,bytes,cut);raise(SIGSTOP);return result;}
 return actual(fd,bytes,size);
}
ssize_t send(int fd,const void *bytes,size_t size,int flags) {
 ssize_t (*actual)(int,const void*,size_t,int)=dlsym(RTLD_NEXT,"send");
 const unsigned char *b=bytes;int cut=0;
 if(fd>2 && mode==1 && size==4 && b[0]==0 && (b[1]||b[2]||b[3]))cut=2;
 if(fd>2 && mode==2 && size>32 && !memcmp(bytes,"{\"worker\":",10))cut=31;
 if(cut){mode=0;ssize_t result=actual(fd,bytes,cut,flags);raise(SIGSTOP);return result;}
 return actual(fd,bytes,size,flags);
}
