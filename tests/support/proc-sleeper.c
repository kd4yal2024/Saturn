/* Test process only: can be copied to any release path without applet dispatch
 * or interpreter-location dependencies. Default signal handling permits cleanup.
 */
#include <unistd.h>

int main(void)
{
    for (;;) {
        pause();
    }
}
