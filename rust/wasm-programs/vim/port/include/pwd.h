/*
 * pwd.h for Vim on an Evans Computer Mod computer: wasi-libc has no users
 * database and a computer has one user. The lookups find nobody
 * (wasi_shim.c), so "~user" stays as typed.
 */
#ifndef ECM_WASI_PWD_H
#define ECM_WASI_PWD_H

#include <sys/types.h>

struct passwd {
    char *pw_name;
    char *pw_passwd;
    uid_t pw_uid;
    gid_t pw_gid;
    char *pw_gecos;
    char *pw_dir;
    char *pw_shell;
};

struct passwd *getpwnam(const char *name);
struct passwd *getpwuid(uid_t uid);
struct passwd *getpwent(void);
void setpwent(void);
void endpwent(void);

#endif
