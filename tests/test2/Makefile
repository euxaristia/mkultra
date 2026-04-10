# Test Makefile 2: Multi-level dependencies and phony targets

all: program

program: main.o utils.o
	echo "Linking program from $^"
	cat $^ > program

main.o: main.c
	echo "Compiling main.c"
	echo "main compiled" > main.o

utils.o: utils.c
	echo "Compiling utils.c"
	echo "utils compiled" > utils.o

utils.c:
	echo "utils source" > utils.c

main.c:
	echo "main source" > main.c

clean:
	rm -f program main.o utils.o main.c utils.c

.PHONY: all clean
