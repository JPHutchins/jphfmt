static PyTypeObject Custom = {
	PyVarObject_HEAD_INIT(NULL, 0)
	.tp_name = "custom.Custom",
	.tp_basicsize = sizeof(CustomObject),
	.tp_new = PyType_GenericNew,
};
