#define RANGE(access_kind, retention_policy, length, \
                    metadata, addr) \
  prefetch(addr, access_kind, retention_policy, metadata)
