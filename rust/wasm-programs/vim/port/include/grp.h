/* grp.h for Vim on an Evans Computer Mod computer: no groups (wasi_shim.c). */
#ifndef ECM_WASI_GRP_H
#define ECM_WASI_GRP_H

#include <sys/types.h>

struct group {
    char *gr_name;
    char *gr_passwd;
    gid_t gr_gid;
    char **gr_mem;
};

struct group *getgrgid(gid_t gid);
struct group *getgrnam(const char *name);

#endif
