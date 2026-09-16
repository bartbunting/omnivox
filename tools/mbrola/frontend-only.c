/* SPDX-License-Identifier: GPL-3.0-or-later
 * Omnivox prototype overlay for eSpeak NG's GPL-3.0-or-later mbrowrap.c.
 * Only produces MBROLA phoneme instructions. Never loads or starts a runtime.
 * The separately verified en1 database has a fixed native rate of 16000 Hz.
 */
#ifdef _WIN32
#include <windows.h>
#endif
#include <stdio.h>
#include <stdlib.h>
#include "mbrowrap.h"

static int WINAPI init(char *path) { (void)path; return 0; }
static void WINAPI noop(void) {}
static void WINAPI volume(float value) { (void)value; }
static void WINAPI tolerance(int value) { (void)value; }
static int WINAPI frequency(void) { return 16000; }
static void unexpected(void) {
    fputs("Omnivox frontend requires --pho -q; PCM calls are forbidden\n", stderr);
    exit(70);
}
static int WINAPI read_pcm(short *buffer, int count) {
    (void)buffer; (void)count; unexpected(); return -1;
}
static int WINAPI write_pcm(char *data) { (void)data; unexpected(); return -1; }
static int WINAPI flush(void) { unexpected(); return 0; }
static char *WINAPI error(char *buffer, int size) {
    if (size > 0) buffer[0] = 0;
    return buffer;
}

int (WINAPI *init_MBR)(char *) = init;
void (WINAPI *close_MBR)(void) = noop;
void (WINAPI *reset_MBR)(void) = noop;
int (WINAPI *read_MBR)(short *, int) = read_pcm;
int (WINAPI *write_MBR)(char *) = write_pcm;
int (WINAPI *flush_MBR)(void) = flush;
int (WINAPI *getFreq_MBR)(void) = frequency;
void (WINAPI *setVolumeRatio_MBR)(float) = volume;
void (WINAPI *setNoError_MBR)(int) = tolerance;
char *(WINAPI *lastErrorStr_MBR)(char *, int) = error;
BOOL load_MBR(void) { return 1; }
void unload_MBR(void) {}
