/* Test-only procfs backend: replace one unrelated process's stat read.
 * All other files, process enumeration and process control stay real.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int open64(const char *path, int flags, ...) {
    int (*real_open)(const char *, int, ...) = dlsym(RTLD_NEXT, "open64");
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list args;
        va_start(args, flags);
        mode = va_arg(args, int);
        va_end(args);
    }
    const char *target = getenv("LOOP_TEST_PROC_STAT");
    if (target && strcmp(path, target) == 0) {
        const char *replacement = getenv("LOOP_TEST_PROC_REPLACEMENT");
        const char *marker = getenv("LOOP_TEST_PROC_MARKER");
        if (!replacement || !marker) _exit(93);
        int seen = real_open(marker, O_WRONLY | O_CREAT | O_APPEND, 0600);
        if (seen < 0 || write(seen, "stat\n", 5) != 5) _exit(94);
        close(seen);
        return real_open(replacement, flags, mode);
    }
    return real_open(path, flags, mode);
}
