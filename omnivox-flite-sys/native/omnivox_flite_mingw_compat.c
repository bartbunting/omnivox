/* SPDX-License-Identifier: MIT */

/*
 * Flite's Windows c99_snprintf helpers are declared with Microsoft's external
 * inline semantics.  MinGW compiles the header as C99, where those declarations
 * need one out-of-line definition.  Include the exact pinned upstream header
 * once with inline removed; ordinary Flite translation units retain their
 * upstream declarations.
 */
#if !defined(__MINGW32__)
#error "omnivox_flite_mingw_compat.c is only for MinGW targets"
#endif

#include <windows.h>

#define __inline
#include "cst_file.h"
