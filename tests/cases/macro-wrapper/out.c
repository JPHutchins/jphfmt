#define DISPATCH_EVENT(handler, event) dispatch_incoming_event( \
	(handler), \
	(event), \
	read_monotonic_timestamp_ms(), \
	current_execution_context_id() \
)
