#define PI 3.14159
#define MIN(a, b) ((a) < (b) ? (a) : (b))
#define LOG(level, fmt, ...) log_impl(level, __FILE__, __LINE__, fmt, __VA_ARGS__)
