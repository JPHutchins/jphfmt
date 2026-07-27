#define RANGE_PREFETCH(access_kind, retention_policy, length, count, stride, metadata, addr) builtin_range_prefetch(addr, access_kind, retention_policy, metadata)
