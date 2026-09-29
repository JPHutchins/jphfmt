struct point {
	int x;
	int y;
};

typedef union {
	int i;
	float f;
} number;

enum color : unsigned char {
	RED,
	GREEN,
};

static int const table[] = {
	1,
	2,
};

struct point origin(void) {
	return (struct point){0, 0};
}

int classify(int n) {
	int total = 0;
	for (int i = 0; i < n; i++) {
		total += i;
	}
	while (total > 100) {
		total /= 2;
	}
	do {
		total++;
	} while (total < 3);
	if (n < 0) {
		return -1;
	} else if (n == 0) {
		return 0;
	} else {
		total--;
	}
	switch (n) {
	case 1: {
		break;
	}
	default:
		break;
	}
	list_for_each(n, total) {
		total++;
	}
	return total;
}
