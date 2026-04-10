# Test Makefile 1: Simple C compilation
CC = gcc
CFLAGS = -Wall

all: hello

hello: hello.o
	$(CC) $(CFLAGS) -o $@ $<

hello.o: hello.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f hello hello.o

.PHONY: all clean
