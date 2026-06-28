/* A deliberately messy but valid C file: collapsed, over-exploded, and mixed
 * indentation, covering every construct cfmt understands. The suite asserts that
 * formatting it is idempotent and changes only whitespace/commas/continuations. */
#include <stdint.h>

#define SQUARE(x) ((x) * (x))
#define CALL_THROUGH(cb, ctx) invoke_registered_callback_with_context((cb), (ctx), default_priority_level(), monotonic_now())

enum state { STATE_IDLE, STATE_RUNNING, STATE_DONE, };

struct config {
    uint32_t flags;
    uint8_t  mode: 2;
};

static int const lookup_table[] = {
    11, 22, 33,
};

int dispatch(int code, int secondary_code, int tertiary_code, int quaternary_code, int quinary_code);

int run(int n) {
        int total = 0;
    struct config const cfg = {.flags = 0, .mode = 1};
    for (int index_variable = 0; index_variable < n && index_variable < lookup_table[0]; index_variable++) {
        if (index_variable == 3 || index_variable == 5 || index_variable == 7 || index_variable == 11 || index_variable == 13) {
            total += dispatch(index_variable, total, cfg.flags, cfg.mode, lookup_table[index_variable % 3]);
        }
    }
    char const * label = (
        total == 0 ? "none" :
        total < 10 ? "few" :
        "many"
    );
    int doubled = ({ int t = total; t * 2; });
    return doubled + (label != nullptr ? 1 : 0);
}
