void f(void) {
	if (
		slots != NULL &&
		match_args != NULL &&
		drop_class_variables(namespace, new_names) == RESULT_OK
	) {
		result = some_function_with_a_fairly_long_name(first_argument_value, second_argument_value);
	}
}
