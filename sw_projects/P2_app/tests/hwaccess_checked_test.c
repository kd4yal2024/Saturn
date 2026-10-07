#define _DEFAULT_SOURCE
#include <assert.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

#include "../../common/hwaccess.h"

extern int register_fd;

int main(void)
{
    uint32_t value = 0xdeadbeefU;
    FILE *file = tmpfile();
    int readonly_fd;

    assert(file != NULL);
    register_fd = fileno(file);
    assert(RegisterWriteChecked(0, 0U));
    assert(RegisterReadChecked(0, &value) && value == 0U);
    assert(!RegisterReadChecked(0, NULL));

    assert(ftruncate(register_fd, 0) == 0);
    value = 0xdeadbeefU;
    assert(!RegisterReadChecked(0, &value));
    assert(value == 0xdeadbeefU);

    register_fd = -1;
    assert(!RegisterReadChecked(0, &value));
    assert(value == 0xdeadbeefU);
    assert(!RegisterWriteChecked(0, 42U));

    readonly_fd = open("/dev/null", O_RDONLY);
    assert(readonly_fd >= 0);
    register_fd = readonly_fd;
    assert(!RegisterWriteChecked(0, 42U));
    close(readonly_fd);
    register_fd = -1;
    fclose(file);
    puts("hwaccess checked-access tests passed");
    return 0;
}
