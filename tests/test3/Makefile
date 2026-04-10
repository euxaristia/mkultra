# Simple test - no variables
all: output.txt

output.txt: input.txt
	cat input.txt > output.txt
	echo "Built output.txt"

input.txt:
	echo "Hello from mkultra" > input.txt

clean:
	rm -f output.txt input.txt

.PHONY: all clean
