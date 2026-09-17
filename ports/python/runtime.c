// ExpPython embedding: each execution owns a fresh, bounded MicroPython VM.
#include <string.h>
#include "py/compile.h"
#include "py/runtime.h"
#include "py/gc.h"
#include "py/cstack.h"
#include "shared/runtime/gchelper.h"

extern void expos_python_output(const char *, size_t);
extern void expos_python_fatal(void) __attribute__((noreturn));
static unsigned char heap[256 * 1024] __attribute__((aligned(16)));
static size_t steps_left, output_left;
static int aborted;

void expos_python_tick(void) {
    if (steps_left) { --steps_left; return; }
    aborted = 1;
    // VM abort cannot be caught by Python try/except and returns to the host.
    nlr_jump_abort();
}

mp_uint_t mp_hal_stdout_tx_strn(const char *str, size_t length) {
    if (length > output_left) { aborted = 2; nlr_jump_abort(); }
    output_left -= length;
    expos_python_output(str, length);
    return length;
}

void gc_collect(void) {
    gc_collect_start();
    gc_helper_collect_regs_and_stack();
    gc_collect_end();
}

void nlr_jump_fail(void *value) {
    (void)value;
    expos_python_fatal();
}

int expos_python_run(const char *source, size_t length) {
    if (length > 4096) return 4;
    mp_cstack_init_with_sp_here(48 * 1024);
    gc_init(heap, heap + sizeof(heap));
    steps_left = 100000;
    output_left = 16384;
    aborted = 0;
    nlr_buf_t boundary;
    int result = 0;
    nlr_set_abort(&boundary);
    if (nlr_push(&boundary) == 0) {
        mp_init();
        mp_lexer_t *lexer = mp_lexer_new_from_str_len(MP_QSTR__lt_stdin_gt_, source, length, 0);
        qstr name = lexer->source_name;
        mp_parse_tree_t tree = mp_parse(lexer, MP_PARSE_FILE_INPUT);
        mp_obj_t function = mp_compile(&tree, name, false);
        mp_call_function_0(function);
        nlr_pop();
    } else {
        result = aborted ? aborted : 3;
        if (!aborted) {
            // Exception printing is bounded too, and may itself abort.
            if (nlr_push(&boundary) == 0) {
                mp_obj_print_exception(&mp_plat_print, MP_OBJ_FROM_PTR(boundary.ret_val));
                nlr_pop();
            }
        }
    }
    nlr_set_abort(NULL);
    mp_deinit();
    memset(heap, 0, sizeof(heap));
    return result;
}

void mp_hal_stdout_tx_strn_cooked(const char *str, size_t length) {
    (void)mp_hal_stdout_tx_strn(str, length);
}
