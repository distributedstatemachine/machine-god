/* ADR 0004: compile-only SDK checks; not a product build input.
 * xcrun clang -arch arm64 -fsyntax-only -Wall -Wextra -Werror FILE
 * xcrun clang -arch x86_64 -fsyntax-only -Wall -Wextra -Werror FILE
 */
#include <stddef.h>
#include <sys/types.h>
#include <sys/sysctl.h>

_Static_assert(sizeof(void *) == 8, "only the verified 64-bit ABI is supported");
_Static_assert(sizeof(struct kinfo_proc) == 648, "Darwin process record stride");
_Static_assert(offsetof(struct kinfo_proc, kp_proc.p_pid) == 40, "Darwin PID offset");
_Static_assert(sizeof(((struct kinfo_proc *)0)->kp_proc.p_pid) == 4, "Darwin PID width");
_Static_assert((pid_t)-1 < 0, "Darwin PID is signed");
_Static_assert(CTL_KERN == 1 && KERN_PROC == 14 && KERN_PROC_ALL == 0,
               "fixed read-only process inventory selector");
