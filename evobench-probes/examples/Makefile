
FLAGS=-std=c++20 -O2 -Wall -Wextra -I../include
OBJS=evobench.o sum_of_fibs.o

EVALUATOR=../../evobench-evaluator/target/release/evobench-evaluator

sum_of_fibs: $(OBJS)
	c++ $(FLAGS) -o $@ $(OBJS)

sum_of_fibs.o: sum_of_fibs.cpp ../include/evobench/*.hpp
	c++ $(FLAGS) -c -o $@ $<

evobench.o: ../src/evobench/evobench.cpp ../include/evobench/*.hpp
	c++ $(FLAGS) -c -o $@ $<

clean:
	rm sum_of_fibs $(OBJS)

bench.log: sum_of_fibs
	EVOBENCH_LOG=bench.log ./sum_of_fibs

run: bench.log

$(EVALUATOR):
	cd ../../evobench-evaluator && cargo build --release

bench.xlsx: bench.log $(EVALUATOR)
	$(EVALUATOR) -d single -e bench.xlsx bench.log

bench-real_time.svg: bench.log $(EVALUATOR)
	$(EVALUATOR) -d single -f bench bench.log
	mv 'bench-real time.svg' bench-real_time.svg
	mv 'bench-sys time.svg' bench-sys_time.svg
	mv 'bench-cpu time.svg' bench-cpu_time.svg
	mv 'bench-ctx switches.svg' bench-ctx_switches.svg

eval: bench.xlsx bench-real_time.svg
