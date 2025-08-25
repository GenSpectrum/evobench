#include <time.h>
#include <unistd.h>
#include <iostream>

#include "evobench/evobench.hpp"

long long fib(long long n) {
   EVOBENCH_SCOPE_EVERY(100000, "fib", "fib");
   if (n <= 2) {
      return n;
   }
   return fib(n - 1) + fib(n - 2);
}

long long sum_of_fibs(long long n) {
    EVOBENCH_SCOPE("sum_of_fibs", "all");
    {
	char buf[30];
	snprintf(buf, 30, "%lld", n);
	EVOBENCH_KEY_VALUE("sum_of_fibs n", buf);
    }
    
    EVOBENCH_SCOPE("sum_of_fibs", "body");
    long long z = 0;
    for (long long i = 0; i < n; i++) {
	EVOBENCH_SCOPE("main", "fib");
	struct timespec req = {
	    .tv_sec = 0,
	    .tv_nsec = 10000000,
	};
	nanosleep(&req, NULL);
	z += fib(i);
    }
    return z;
}

int main() {
    EVOBENCH_SCOPE("main", "main");
    sleep(1);
    for (long long i = 0; i < 40; i++) {
	std::cout << "sum_of_fibs(" << i << ") = " << sum_of_fibs(i) << "\n";
    }
    return 0;
}
