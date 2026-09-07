// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Test-only C implementations validate the actual dynamic ABI without runtimes.
#define _DEFAULT_SOURCE
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <stdio.h>

typedef int (*EciCallback)(void *, int, long, void *);
static pthread_t owner;
static int16_t *eci_buffer;
static EciCallback eci_callback;
static void *eci_data;
static uint32_t indexes[4096], positions[4096], index_count, text_bytes;
void eciVersion(char *version) { strcpy(version,"ECI ABI test stub"); }
void *eciNewEx(int dialect) { owner=pthread_self(); return dialect==0x10000 ? (void *)1 : NULL; }
void eciDelete(void *h) { (void)h; if (!pthread_equal(owner,pthread_self())) abort(); }
int eciStop(void *h) { (void)h; if (!pthread_equal(owner,pthread_self())) abort(); return 1; }
int eciClearInput(void *h) {
 eciStop(h);
 index_count=0; text_bytes=0;
 return getenv("OMNIVOX_STUB_VOXIN_PATCH") ? 0 : 1;
}
int voxGetVersion(int *major, int *minor, int *patch) {
 const char *value=getenv("OMNIVOX_STUB_VOXIN_PATCH");
 if (!value) return -1;
 *major=1; *minor=6; *patch=atoi(value); return 0;
}
int eciSetParam(void *h,int p,int v) { (void)h;(void)p;(void)v;return 1; }
void eciRegisterCallback(void *h,EciCallback callback,void *data) { (void)h;eci_callback=callback;eci_data=data; }
int eciSetOutputBuffer(void *h,int size,int16_t *buffer) { (void)h;eci_buffer=buffer;return size==512; }
int eciAddText(void *h,const char *text) {
 (void)h;
 if (!text || !*text) return 0;
 if (text[0]==' ' && text[1]=='`') return 1;
 text_bytes+=(uint32_t)strlen(text); return 1;
}
int eciInsertIndex(void *h,int index) {
 (void)h; if(index_count>=4096) return 0;
 indexes[index_count]=index; positions[index_count++]=text_bytes*32; return 1;
}
int eciSynthesize(void *h) { return eciStop(h); }
int eciSynchronize(void *h) {
 if (!pthread_equal(owner,pthread_self())) abort();
 uint32_t cursor=0,next=0;
 while(cursor<24576) {
  while(next<index_count && positions[next]==cursor) {
   if (!getenv("OMNIVOX_STUB_OMIT_INDEX")) {
    if(eci_callback(h,2,indexes[next],eci_data)==2) return 1;
    if(getenv("OMNIVOX_STUB_DUPLICATE_INDEX") && eci_callback(h,2,indexes[next],eci_data)==2) return 1;
   }
   next++;
  }
  uint32_t size=512;
  if(next<index_count && positions[next]-cursor<size) size=positions[next]-cursor;
  if(24576-cursor<size) size=24576-cursor;
  for(uint32_t i=0;i<size;i++) eci_buffer[i]=(i%64-32)*400;
  if(eci_callback(h,0,getenv("OMNIVOX_STUB_BAD_PCM")?513:size,eci_data)==2) return 1;
  cursor+=size;
  usleep(1000);
 }
 return 1;
}

typedef struct {
 int16_t *data; void *phonemes; void *indexes;
 uint32_t max_bytes,max_phonemes,max_indexes,bytes,phonemes_count,index_count,reserved;
} Buffer;
typedef struct { uint32_t value,sample,reserved; } Index;
typedef struct { uint32_t phoneme,sample,duration,reserved; } Phoneme;
typedef void (*DtkCallback)(long,long,uint32_t,uint32_t);
static DtkCallback dtk_callback;
static Buffer *dtk_buffers[4];
static int buffer_count;
static atomic_int stopped;
uint32_t TextToSpeechVersion(const char **version) { *version="DECtalk ABI test stub";return 0x04990000; }
uint32_t TextToSpeechStartupExFonix(void **h,uint32_t device,uint32_t flags,DtkCallback callback,long data,const char *dictionary) {
 (void)device;(void)data;
 if(flags!=0x80000000 || !dictionary || dictionary[0]!='/') return 1;
 *h=(void *)1;dtk_callback=callback;return 0;
}
uint32_t TextToSpeechOpenInMemory(void *h,uint32_t format) { (void)h;return format==4?0:1; }
uint32_t TextToSpeechAddBuffer(void *h,Buffer *buffer) {
 (void)h;
 for(int i=0;i<buffer_count;i++) if(dtk_buffers[i]==buffer) return 0;
 if(buffer_count>=4 || buffer->max_bytes!=1024) return 1;
 dtk_buffers[buffer_count++]=buffer;return 0;
}
uint32_t TextToSpeechReset(void *h,uint8_t modes) { (void)h;(void)modes;atomic_store(&stopped,1);return 0; }
uint32_t TextToSpeechSetRate(void *h,uint32_t rate) { (void)h;return rate>=75 && rate<=600?0:1; }
uint32_t TextToSpeechSpeak(void *h,const char *text,uint32_t flags) {
 (void)h;(void)flags;atomic_store(&stopped,0);index_count=0;text_bytes=0;
 for(const char *p=text;*p;) {
  if(*p=='[') {
   unsigned value;
   if(sscanf(p,"[:index mark %u]",&value)==1 && index_count<4096) {
    indexes[index_count]=value; positions[index_count++]=text_bytes*32;
   }
   const char *end=strchr(p,']'); if(!end) return 1;
   p=end+1;
   if(text_bytes==0 && index_count==0 && *p==' ') p++;
  } else { text_bytes++;p++; }
 }
 return 0;
}
uint32_t TextToSpeechSync(void *h) {
 (void)h;
 uint32_t next=0;
 for(int n=0;n<48 && !atomic_load(&stopped);n++) {
  Buffer *b=dtk_buffers[n%4];
  for(int i=0;i<512;i++) b->data[i]=(i%64-32)*400;
  b->bytes=getenv("OMNIVOX_STUB_BAD_PCM")?1026:1024;
  b->index_count=0;
  /* Deliberately report some indexes in the following PCM callback. */
  while(next<index_count && positions[next]<(uint32_t)n*512) {
   if(b->index_count>=b->max_indexes) abort();
   Index *index=(Index *)b->indexes+b->index_count++;
   index->value=indexes[next];index->sample=positions[next++];
  }
  b->phonemes_count=1;
  Phoneme *phoneme=b->phonemes;
  phoneme->phoneme=42;phoneme->sample=n?(uint32_t)n*512-8:0;
  dtk_callback(0,(long)b,0,9);
  usleep(1000);
 }
 return 0;
}
uint32_t TextToSpeechCloseInMemory(void *h) { (void)h;return 0; }
uint32_t TextToSpeechShutdown(void *h) { (void)h;return 0; }
