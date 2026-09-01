/* SPDX-License-Identifier: MIT */

/*
 * Flite declares several Windows helpers with Microsoft's external inline
 * semantics. MinGW compiles them as C99, where those declarations need an
 * out-of-line definition in unoptimized builds. Include the exact pinned file
 * helper header once with inline removed, then provide the two UTF-8 helpers
 * whose definitions live only in upstream C translation units. Ordinary Flite
 * translation units retain their upstream declarations and implementations.
 */
#if !defined(__MINGW32__)
#error "omnivox_flite_mingw_compat.c is only for MinGW targets"
#endif

#include <windows.h>

#define __inline
#include "cst_file.h"

int utf8_sequence_length(char c0)
{
    return ((0xE5000000 >> ((c0 >> 3) & 0x1E)) & 3) + 1;
}

int ts_utf8_sequence_length(char c0)
{
    return ((0xE5000000 >> ((c0 >> 3) & 0x1E)) & 3) + 1;
}
