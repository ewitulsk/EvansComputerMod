/*
 * wasi_shim.c: the OS pieces Vim needs that wasi-libc doesn't have, for Vim
 * on an Evans Computer Mod computer. See wasi_shim.h for the overview.
 */
#include "auto/config.h"

#include <errno.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <time.h>
#include <wasi/api.h>

/* The real calls (wasi_shim.h renames them for Vim's sources). */
#undef read
#undef poll
#undef ioctl
#undef fork
#undef system
#undef dup
#undef kill
#undef stat
#undef lstat
#undef fstat
#undef isatty

/* --- terminal settings and size ---------------------------------------- */

static struct termios tty_settings = {
    .c_iflag = ICRNL | IXON,
    .c_oflag = OPOST | ONLCR,
    .c_cflag = CS8 | CREAD,
    .c_lflag = ISIG | ICANON | ECHO | ECHOE | ECHOK | IEXTEN,
    .c_cc = {3, 28, 127, 21, 4, 0, 1, 0, 17, 19, 26},
    .c_ispeed = B38400,
    .c_ospeed = B38400,
};
static int reports_on;
static int own_ctrl_keys;
static int rows, cols;

int wasi_shim_isatty(int fd) {
    __wasi_fdstat_t st;
    if (__wasi_fd_fdstat_get(fd, &st) != 0) {
        errno = EBADF;
        return 0;
    }
    if (st.fs_filetype != __WASI_FILETYPE_CHARACTER_DEVICE) {
        errno = ENOTTY;
        return 0;
    }
    return 1;
}

static void put(const char *s) {
    size_t n = strlen(s);
    while (n > 0) {
        ssize_t w = write(1, s, n);
        if (w <= 0) return;
        s += w;
        n -= (size_t)w;
    }
}

static void init_size(void) {
    if (rows > 0) return;
    const char *l = getenv("LINES"), *c = getenv("COLUMNS");
    rows = l ? atoi(l) : 0;
    cols = c ? atoi(c) : 0;
    if (rows <= 0) rows = 25;
    if (cols <= 0) cols = 80;
}

int tcgetattr(int fd, struct termios *t) {
    if (!wasi_shim_isatty(fd)) {
        errno = ENOTTY;
        return -1;
    }
    *t = tty_settings;
    return 0;
}

int tcsetattr(int fd, int when, const struct termios *t) {
    (void)when;
    if (!wasi_shim_isatty(fd)) {
        errno = ENOTTY;
        return -1;
    }
    tty_settings = *t;
    /* Raw (Vim is editing): follow the window size through in-band resize
     * reports; cooked (Vim is leaving or suspended): stop them. */
    int raw = !(t->c_lflag & ICANON);
    if (raw != reports_on) {
        reports_on = raw;
        put(raw ? "\033[?2048h" : "\033[?2048l");
    }
    /* ISIG off: Ctrl+C and Ctrl+Z are keys for Vim, not the shell's interrupt
     * and suspend. The computer's terminal has a mode for that (?7727); without
     * it only the alternate screen gives them to Vim, and Vim leaves that
     * screen for a moment around :! and the hit-enter prompt. */
    /* (Vim's sleep mode keeps ISIG for SIGINT, which doesn't exist here: Vim
     * reads Ctrl+C there too, so the keys stay Vim's unless the tty is cooked.) */
    int keys_are_ours = !(t->c_lflag & ISIG) || !(t->c_lflag & ICANON);
    if (keys_are_ours != own_ctrl_keys) {
        own_ctrl_keys = keys_are_ours;
        put(keys_are_ours ? "\033[?7727h" : "\033[?7727l");
    }
    return 0;
}

speed_t cfgetospeed(const struct termios *t) { return t->c_ospeed; }
speed_t cfgetispeed(const struct termios *t) { return t->c_ispeed; }

int wasi_shim_ioctl(int fd, int request, ...) {
    va_list ap;
    va_start(ap, request);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    if (request == TIOCGWINSZ) {
        if (!wasi_shim_isatty(fd)) {
            errno = ENOTTY;
            return -1;
        }
        init_size();
        struct winsize *ws = arg;
        ws->ws_row = (unsigned short)rows;
        ws->ws_col = (unsigned short)cols;
        ws->ws_xpixel = ws->ws_ypixel = 0;
        return 0;
    }
    if (request == TIOCSWINSZ) return 0;
    errno = ENOTTY;
    return -1;
}

/* --- keyboard input ---------------------------------------------------- */

/* Keys read from the tty but not yet given to Vim (grown dynamically up to 1 MiB
 * so large pastes are not silently truncated). */
#define MAX_KEYS_CAP (1024 * 1024)
static unsigned char *keys;
static size_t nkeys;
static size_t capkeys;
/* The start of a resize report that ended mid-read: ESC [ 4 8 ; ... */
static char partial[64];
static size_t npartial;

static void push_key(unsigned char b) {
    if (nkeys == capkeys) {
        size_t next = capkeys == 0 ? 8192 : capkeys * 2;
        if (next > MAX_KEYS_CAP) next = MAX_KEYS_CAP;
        if (nkeys >= next) return;
        unsigned char *p = realloc(keys, next);
        if (!p) return;
        keys = p;
        capkeys = next;
    }
    keys[nkeys++] = b;
}

static int more_ready(int fd) {
    struct pollfd pfd;
    pfd.fd = fd;
    pfd.events = POLLIN;
    pfd.revents = 0;
    return poll(&pfd, 1, 0) > 0 && (pfd.revents & POLLIN);
}

/* Does `s` (n bytes) start a report "ESC[48;r;c;h;wt"? 2 = a whole one
 * (its length in *len), 1 = maybe (too short to tell), 0 = no. */
static int match_report(const unsigned char *s, size_t n, size_t *len, int *r, int *c) {
    static const char head[] = "\033[48;";
    size_t i = 0;
    for (; i < 5; i++) {
        if (i >= n) return 1;
        if (s[i] != (unsigned char)head[i]) return 0;
    }
    int nums[5] = {0}, k = 0;
    for (; i < n; i++) {
        unsigned char b = s[i];
        if (b >= '0' && b <= '9') {
            if (nums[k] < 100000) nums[k] = nums[k] * 10 + (b - '0');
        } else if (b == ';') {
            if (++k >= 5) return 0;
        } else if (b == 't') {
            *len = i + 1;
            *r = nums[0];
            *c = nums[1];
            return 2;
        } else {
            return 0;
        }
        if (i > 40) return 0;
    }
    return 1;
}

/* Take resize reports out of `data`, keep the keys. Returns 1 if the size changed. */
static int take_keys(int fd, const unsigned char *data, size_t n) {
    unsigned char buf[sizeof(partial) + 4096];
    size_t m = 0;
    memcpy(buf, partial, npartial);
    m = npartial;
    npartial = 0;
    if (n > sizeof(buf) - m) n = sizeof(buf) - m;
    memcpy(buf + m, data, n);
    m += n;

    int resized = 0;
    size_t i = 0;
    while (i < m) {
        if (buf[i] == 0x1b) {
            size_t len;
            int r, c;
            int k = match_report(buf + i, m - i, &len, &r, &c);
            if (k == 2) {
                if (r > 0 && c > 0 && (r != rows || c != cols)) {
                    rows = r;
                    cols = c;
                    resized = 1;
                }
                i += len;
                continue;
            }
            /* Hold back an incomplete "ESC[48;..." prefix at the end of a read.
             * Also hold back a 2-4 byte prefix ("ESC[", "ESC[4", "ESC[48") when
             * more bytes are already waiting in the pipe, so a split resize report
             * is reassembled without delaying a lone Esc key. */
            if (k == 1 && m - i < sizeof(partial) && (m - i >= 5 || (m - i >= 2 && more_ready(fd)))) {
                memcpy(partial, buf + i, m - i);
                npartial = m - i;
                break;
            }
        }
        unsigned char b = buf[i];
        /* The computer's keyboard sends Enter as "\n" (the byte the shell
         * reads). A raw tty without ICRNL sees the Enter key as "\r", and Vim
         * reads "\n" as Ctrl+J (down a line), so it gets "\r". */
        if (b == '\n' && !(tty_settings.c_iflag & ICRNL) && !(tty_settings.c_lflag & ICANON)) b = '\r';
        push_key(b);
        i++;
    }
    if (resized) raise(SIGWINCH);
    return resized;
}

static size_t give_keys(void *out, size_t n) {
    if (n > nkeys) n = nkeys;
    memcpy(out, keys, n);
    memmove(keys, keys + n, nkeys - n);
    nkeys -= n;
    return n;
}

static long long monotonic_ms(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) return 0;
    return (long long)ts.tv_sec * 1000LL + (long long)(ts.tv_nsec / 1000000L);
}

ssize_t wasi_shim_read(int fd, void *buf, size_t n) {
    if (!wasi_shim_isatty(fd)) return read(fd, buf, n);
    for (;;) {
        if (nkeys > 0) return (ssize_t)give_keys(buf, n);
        unsigned char tmp[4096];
        ssize_t got = read(fd, tmp, sizeof(tmp));
        if (got <= 0) return got;
        init_size();
        take_keys(fd, tmp, (size_t)got);
    }
}

int wasi_shim_poll(struct pollfd *fds, nfds_t n, int timeout) {
    int tty_at = -1;
    for (nfds_t i = 0; i < n; i++) {
        fds[i].revents = 0;
        if (tty_at < 0 && (fds[i].events & POLLIN) && wasi_shim_isatty(fds[i].fd)) tty_at = (int)i;
    }
    if (tty_at < 0) return poll(fds, n, timeout);
    if (nkeys > 0) {
        fds[tty_at].revents = POLLIN;
        return 1;
    }
    long long deadline = timeout > 0 ? monotonic_ms() + (long long)timeout : 0;
    for (;;) {
        int rem = timeout;
        if (timeout > 0) {
            long long left = deadline - monotonic_ms();
            if (left <= 0) return 0;
            rem = left > 0x7fffffffLL ? 0x7fffffff : (int)left;
        }
        int ret = poll(fds, n, rem);
        if (ret <= 0 || !(fds[tty_at].revents & (POLLIN | POLLHUP))) return ret;
        unsigned char tmp[4096];
        ssize_t got = read(fds[tty_at].fd, tmp, sizeof(tmp));
        if (got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            fds[tty_at].revents = 0;
            if (--ret > 0) return ret;
            if (timeout == 0) return 0;
            continue;
        }
        if (got <= 0) {
            fds[tty_at].revents = POLLIN | POLLHUP; /* end of input: Vim's read sees it */
            return ret;
        }
        init_size();
        int resized = take_keys(fds[tty_at].fd, tmp, (size_t)got);
        if (nkeys > 0) {
            fds[tty_at].revents = POLLIN;
            return ret;
        }
        /* Only a resize report (or partial): like a tty's SIGWINCH, a real size
         * change interrupts the wait; a no-op report matching the current size
         * keeps waiting for the rest of the timeout instead of timing out early. */
        fds[tty_at].revents = 0;
        if (--ret > 0) return ret;
        if (resized) {
            errno = EINTR;
            return -1;
        }
    }
}

/* --- file permissions ------------------------------------------------- */

static int read_only_path(const char *path) {
    while (path[0] == '.' && path[1] == '/') path += 2;
    while (*path == '/') path++;
    return strncmp(path, "server-bin", 10) == 0 && (path[10] == '\0' || path[10] == '/');
}

static void add_perms(struct stat *st, const char *path) {
    /* WasiFunctions.writeFilestat sets st_dev = 2 for /server-bin (read-only)
     * and 1 for computer storage, so this stays accurate for relative paths,
     * '..' segments and fstat(). */
    int read_only = (st->st_dev == 2) || (st->st_dev == 0 && path != NULL && read_only_path(path));
    mode_t type = st->st_mode & S_IFMT;
    mode_t perm = (type == S_IFDIR) ? 0755 : 0644;
    if (read_only) perm &= ~0222;
    st->st_mode = type | perm;
}

int wasi_shim_stat(const char *path, struct stat *st) {
    int r = stat(path, st);
    if (r == 0) add_perms(st, path);
    return r;
}

int wasi_shim_lstat(const char *path, struct stat *st) {
    int r = lstat(path, st);
    if (r == 0) add_perms(st, path);
    return r;
}

int wasi_shim_fstat(int fd, struct stat *st) {
    int r = fstat(fd, st);
    if (r == 0) add_perms(st, NULL);
    return r;
}

/* --- what WASI doesn't have -------------------------------------------- */

pid_t wasi_shim_fork(void) {
    errno = ENOSYS;
    return -1;
}

int wasi_shim_system(const char *cmd) {
    if (cmd == NULL) return 0; /* no shell */
    errno = ENOSYS;
    return -1;
}

pid_t waitpid(pid_t pid, int *status, int options) {
    (void)pid; (void)status; (void)options;
    errno = ECHILD;
    return -1;
}

int pipe(int fds[2]) {
    (void)fds;
    errno = ENOSYS;
    return -1;
}

int execvp(const char *file, char *const argv[]) {
    (void)file; (void)argv;
    errno = ENOSYS;
    return -1;
}

int setsid(void) {
    errno = ENOSYS;
    return -1;
}

int wasi_shim_dup(int fd) {
    (void)fd;
    errno = ENOSYS;
    return -1;
}

int wasi_shim_kill(pid_t pid, int sig) {
    /* No other process Vim could signal or find (an old swap file's owner is gone). */
    (void)pid;
    (void)sig;
    errno = ESRCH;
    return -1;
}

int gethostname(char *name, size_t len) {
    const char *h = getenv("HOSTNAME");
    if (h == NULL || *h == '\0') h = "computer";
    if (len == 0) return 0;
    strncpy(name, h, len - 1);
    name[len - 1] = '\0';
    return 0;
}

/* pwd.h: one user and no database. */
#include <pwd.h>
struct passwd *getpwnam(const char *name) { (void)name; return NULL; }
struct passwd *getpwuid(uid_t uid) { (void)uid; return NULL; }
struct passwd *getpwent(void) { return NULL; }
void setpwent(void) {}
void endpwent(void) {}

/* grp.h: no groups. */
#include <grp.h>
struct group *getgrgid(gid_t gid) { (void)gid; return NULL; }
struct group *getgrnam(const char *name) { (void)name; return NULL; }
