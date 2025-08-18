
FLAGS=-std=c++20 -O2 -Wall -Wextra -I../include
OBJS=evobench.o sum_of_fibs.o

sum_of_fibs: $(OBJS)
	c++ $(FLAGS) -o $@ $(OBJS)

sum_of_fibs.o: sum_of_fibs.cpp ../include/evobench/*.hpp
	c++ $(FLAGS) -c -o $@ $<

evobench.o: ../src/evobench/evobench.cpp ../include/evobench/*.hpp
	c++ $(FLAGS) -c -o $@ $<

clean:
	rm sum_of_fibs $(OBJS)

run: sum_of_fibs
	EVOBENCH_LOG=bench.log ./sum_of_fibs
