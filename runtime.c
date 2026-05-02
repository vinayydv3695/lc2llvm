#include <stdint.h>
#include <stdio.h>

typedef struct {
    void* fn_ptr;
    void* env;
} Closure;

void print_int(long x) {
    printf("%ld\n", x);
}
