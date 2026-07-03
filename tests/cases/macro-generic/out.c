#define type_name(x) _Generic( \
	(x), \
	int: "int", \
	long: "long", \
	float: "float", \
	double: "double", \
	default: "other" \
)
