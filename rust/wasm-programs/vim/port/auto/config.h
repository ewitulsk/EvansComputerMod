/*
 * auto/config.h for Vim on an Evans Computer Mod computer: wasm32-wasip1,
 * built with wasi-sdk (rust/wasm-programs/vim/build.rs). Written by hand in
 * place of the one `configure` generates: there is no configure run for WASI.
 *
 * The computer has a terminal (the kernel's VTE, an xterm subset), files, a
 * clock and environment variables. It has no processes Vim can fork or wait
 * for, no signals from outside, no termios, no users database, no X11, no
 * sockets Vim knows how to use. Those features are left out here and the
 * rest come from wasi_shim.h.
 */

#define UNIX 1
#define FEAT_HUGE 1
#define HAVE_ATTRIBUTE_UNUSED 1

#define VIM_SIZEOF_INT 4
#define VIM_SIZEOF_LONG 4
#define SIZEOF_OFF_T 8
#define SIZEOF_TIME_T 8
#define SIZEOF_WCHAR_T 4

#define USEMEMMOVE 1
#define SYS_SELECT_WITH_SYS_TIME 1
#define SELECT_TYPE_ARG234 (fd_set *)
#define TIME_WITH_SYS_TIME 1
#define ST_MTIM_NSEC st_mtim.tv_nsec

/* Functions wasi-libc has. */
#define HAVE_FSEEKO 1
#define HAVE_FSYNC 1
#define HAVE_FTRUNCATE 1
#define HAVE_GETCWD 1
#define HAVE_GETTIMEOFDAY 1
#define HAVE_LOCALTIME_R 1
#define HAVE_LSTAT 1
#define HAVE_MEMSET 1
#define HAVE_NANOSLEEP 1
#define HAVE_NL_LANGINFO_CODESET 1
#define HAVE_OPENDIR 1
#define HAVE_PUTENV 1
#define HAVE_QSORT 1
#define HAVE_READLINK 1
#define HAVE_RENAME 1
/* No HAVE_SELECT: Vim waits for keys with poll(), which wasi_shim.h wraps. */
#define HAVE_SETENV 1
#define HAVE_STRCASECMP 1
#define HAVE_STRCOLL 1
#define HAVE_STRERROR 1
#define HAVE_STRFTIME 1
#define HAVE_STRNCASECMP 1
#define HAVE_STRPBRK 1
#define HAVE_STRTOL 1
#define HAVE_ST_BLKSIZE 1
#define HAVE_SYSCONF 1
#define HAVE_TOWLOWER 1
#define HAVE_TOWUPPER 1
#define HAVE_ISWUPPER 1
#define HAVE_UNSETENV 1
#define HAVE_USLEEP 1
#define HAVE_UTIMES 1
#define HAVE_MBLEN 1
#define HAVE_CLOCK_GETTIME 1
#define HAVE_ISINF 1
#define HAVE_ISNAN 1
#define HAVE_DIRFD 1

/* Headers wasi-libc has. termios.h is the port's own (wasi_shim.h). */
#define HAVE_DIRENT_H 1
#define HAVE_ERRNO_H 1
#define HAVE_FCNTL_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_LANGINFO_H 1
#define HAVE_LIBGEN_H 1
#define HAVE_LOCALE_H 1
#define HAVE_MATH_H 1
#define HAVE_POLL_H 1
#define HAVE_STDINT_H 1
#define HAVE_STRINGS_H 1
#define HAVE_SYS_IOCTL_H 1
#define HAVE_SYS_PARAM_H 1
#define HAVE_SYS_POLL_H 1
#define HAVE_SYS_SELECT_H 1
#define HAVE_SYS_TIME_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_TERMIOS_H 1
#define HAVE_WCHAR_H 1
#define HAVE_WCTYPE_H 1
#define HAVE_UNISTD_H 1
#define HAVE_UTIME_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRING_H 1

/* Every feature that needs X11, a GUI, other processes, sockets or shared
 * libraries stays off: FEAT_JOB_CHANNEL, FEAT_TERMINAL, FEAT_NETBEANS_INTG,
 * FEAT_CLIENTSERVER, the language interfaces, HAVE_DLOPEN. */
#define DISABLE_RIGHTLEFT 1
#define DISABLE_ARABIC 1
#define MODIFIED_BY "Evans Computer Mod (WASI port)"

/* The port's replacements for termios, ioctl, fork and friends. */
#include "wasi_shim.h"
