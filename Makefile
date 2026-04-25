.PHONY: all clean test install uninstall

BINDIR = $(HOME)/.local/bin

all:
	ponyc src -b mkultra -o .

clean:
	rm -f mkultra mkultra.o

test:
	ponyc src --debug -b mkultra-test -o . && ./mkultra-test

install: all
	install -Dm755 mkultra $(BINDIR)/mkultra

uninstall:
	rm -f $(BINDIR)/mkultra
