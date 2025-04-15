
OBJS=evobench.o sum_of_fibs.o

sum_of_fibs: $(OBJS)
	c++ -std=c++17 -O2 -Wall -Wextra -o $@ $(OBJS)

sum_of_fibs.o: sum_of_fibs.cpp ../include/evobench/*.hpp
	c++ -std=c++17 -O2 -Wall -Wextra -c -o $@ $<

evobench.o: ../src/evobench/evobench.cpp ../include/evobench/*.hpp
	c++ -std=c++17 -O2 -Wall -Wextra -c -o $@ $<

clean:
	rm sum_of_fibs $(OBJS)

run: sum_of_fibs
	EVOBENCH_LOG=bench.log ./sum_of_fibs
