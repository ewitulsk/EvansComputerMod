/*
 * wasi_shim.h: what Vim's Unix code expects from the OS and wasi-libc lacks,
 * for Vim on an Evans Computer Mod computer (wasm32-wasip1). Included from
 * auto/config.h, so every Vim source file sees it first.
 *
 * - The terminal: the computer's text console is always in "raw" mode for a
 *   running program (no echo, no line editing; the kernel only keeps Ctrl+T).
 *   tcgetattr/tcsetattr keep a struct termios so Vim's mode switching works,
 *   and switching modes turns the terminal's in-band resize reports on and
 *   off (ESC[?2048h). The window size is COLUMNS/LINES at start, then
 *   whatever the reports say; TIOCGWINSZ answers it.
 * - Keyboard input: read() and poll() on stdin go through the shim, which
 *   takes the resize reports out of the key stream and raises SIGWINCH
 *   (wasi-libc's emulated signals run the handler at once), the way a real
 *   tty sends SIGWINCH.
 * - Other processes: there are none Vim can start. fork() and system() fail
 *   with ENOSYS, and the shell commands give a clear error (os_unix.c).
 */
#ifndef ECM_WASI_SHIM_H
#define ECM_WASI_SHIM_H

#include <sys/types.h>
#include <sys/stat.h>
#include <unistd.h>
#include <poll.h>
#include <signal.h>
#include <stddef.h>

/* Vim uses system() for shell commands (os_unix.c USE_SYSTEM), never fork(). */
#define USE_SYSTEM 1

/* --- the terminal ------------------------------------------------------- */

struct winsize {
    unsigned short ws_row;
    unsigned short ws_col;
    unsigned short ws_xpixel;
    unsigned short ws_ypixel;
};
#define TIOCGWINSZ 0x5413
#define TIOCSWINSZ 0x5414
int wasi_shim_ioctl(int fd, int request, ...);
#define ioctl(...) wasi_shim_ioctl(__VA_ARGS__)

/* wasi-libc's isatty() also wants the fd without seek rights, which the
 * computer's console doesn't drop; a character device is the terminal. */
int wasi_shim_isatty(int fd);
#define isatty(fd) wasi_shim_isatty(fd)

/* --- keyboard input ----------------------------------------------------- */

ssize_t wasi_shim_read(int fd, void *buf, size_t n);
int wasi_shim_poll(struct pollfd *fds, nfds_t n, int timeout);
#define read(fd, buf, n) wasi_shim_read(fd, buf, n)
#define poll(fds, n, timeout) wasi_shim_poll(fds, n, timeout)

/* --- file permissions ---------------------------------------------------- */

/* wasi-libc's stat() gives only the file type. Vim reads the permission bits
 * (a file without write bits opens 'readonly'), so the shim adds them: the
 * server's program folder (/server-bin) is read-only, the computer's own
 * files are the user's to write. */
int wasi_shim_stat(const char *path, struct stat *st);
int wasi_shim_lstat(const char *path, struct stat *st);
int wasi_shim_fstat(int fd, struct stat *st);
#define stat(path, st) wasi_shim_stat(path, st)
#define lstat(path, st) wasi_shim_lstat(path, st)
#define fstat(fd, st) wasi_shim_fstat(fd, st)

/* --- processes, users and permissions WASI doesn't have ------------------ */

pid_t wasi_shim_fork(void);
#define fork() wasi_shim_fork()
int wasi_shim_system(const char *cmd);
#define system(cmd) wasi_shim_system(cmd)
#define dup(fd) wasi_shim_dup(fd)
/* Never reached (fork fails first), but Vim's Unix code names them. */
#define WNOHANG 1
#define WIFEXITED(s) (((s) & 0x7f) == 0)
#define WEXITSTATUS(s) (((s) >> 8) & 0xff)
#define WIFSIGNALED(s) 0
#define WTERMSIG(s) 0
pid_t waitpid(pid_t pid, int *status, int options);
int pipe(int fds[2]);
int execvp(const char *file, char *const argv[]);
int setsid(void);
int wasi_shim_dup(int fd);
#define kill(pid, sig) wasi_shim_kill(pid, sig)
int wasi_shim_kill(pid_t pid, int sig);

#define getuid() ((uid_t)0)
#define geteuid() ((uid_t)0)
#define getgid() ((gid_t)0)
#define getegid() ((gid_t)0)
#define umask(m) ((mode_t)022)
#define chmod(path, mode) (0)
#define fchmod(fd, mode) (0)
#define fchown(fd, u, g) (0)
#define chown(path, u, g) (0)
#define lchown(path, u, g) (0)
int gethostname(char *name, size_t len);

/* The program's entry (main.rs calls it). */
int vim_main(int argc, char **argv);

#endif
