#include <iostream>

#include "../src/evobench.cpp" // XXX

long long fib(long long n) {
    // EVOBENCH_SCOPE("fib", "fib");
    if (n <= 2) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

long long sum_of_fibs(long long n) {
    {
        char buf[30];
        snprintf(buf, 30, "%lld", n);
        EVOBENCH_KEY_VALUE("sum_of_fibs n", buf);
    }

    EVOBENCH_SCOPE("main", "sum_of_fibs");
    long long z = 0;
    for (long long i = 0; i < n; i++) {
        EVOBENCH_SCOPE("main", "fib");
        z += fib(i);
    }
    return z;
}

int main() {
    EVOBENCH_SCOPE("main", "main");
    for (long long i = 0; i < 40; i++) {
        std::cout << "sum_of_fibs(" << i << ") = " << sum_of_fibs(i) << "\n";
    }
    return 0;
}
