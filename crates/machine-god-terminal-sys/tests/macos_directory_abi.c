/* ADR 0005: SDK layout and fixed-symbol link check; not a product build input.
 * xcrun clang -arch arm64 -Wall -Wextra -Werror FILE -o OUTPUT
 * xcrun clang -arch x86_64 -Wall -Wextra -Werror FILE -o OUTPUT
 */
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>
#include <dirent.h>

/* Apple Libc gen/FreeBSD/telldir.h's fixed exported declaration. */
extern size_t __getdirentries64(int, void *, size_t, off_t *);

_Static_assert(sizeof(size_t) == 8 && sizeof(off_t) == 8, "64-bit scalar ABI");
_Static_assert(sizeof(struct dirent) == 1048, "64-bit directory record layout");
_Static_assert(offsetof(struct dirent, d_ino) == 0, "inode offset");
_Static_assert(offsetof(struct dirent, d_reclen) == 16, "record length offset");
_Static_assert(offsetof(struct dirent, d_namlen) == 18, "name length offset");
_Static_assert(offsetof(struct dirent, d_type) == 20, "type offset");
_Static_assert(offsetof(struct dirent, d_name) == 21, "name offset");
_Static_assert(sizeof(((struct dirent *)0)->d_ino) == 8, "inode width");
_Static_assert(sizeof(((struct dirent *)0)->d_reclen) == 2, "record length width");
_Static_assert(sizeof(((struct dirent *)0)->d_namlen) == 2, "name length width");
_Static_assert(sizeof(((struct dirent *)0)->d_name) == 1024, "name storage bound");

int main(void) {
    unsigned char buffer[8192] = {0};
    off_t base = 0;
    /* Invalid descriptor makes the link probe safe to execute, without paths. */
    return __getdirentries64(-1, buffer, sizeof(buffer), &base) == SIZE_MAX ? 0 : 1;
}
