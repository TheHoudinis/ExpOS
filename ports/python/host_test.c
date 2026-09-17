#include <stdio.h>
#include <stdlib.h>
#include <string.h>
void expos_python_output(const char *source, size_t length) { fwrite(source, 1, length, stdout); }
void expos_python_fatal(void) { abort(); }
extern int expos_python_run(const char *, size_t);
int main(int argc, char **argv) {
    if (argc < 2) return 99;
    int result = 0;
    for (int i = 1; i < argc; i++) result = expos_python_run(argv[i], strlen(argv[i]));
    return result;
}
