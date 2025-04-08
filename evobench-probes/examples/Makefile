
sum_of_fibs: sum_of_fibs.cpp ../src/evobench.cpp ../include/evobench.hpp
	c++ -std=c++17 -O2 -Wall -Wextra -o $@ $<

run: sum_of_fibs
	EVOBENCH_LOG=bench.log ./sum_of_fibs
