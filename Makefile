.PHONY: all clean test install uninstall

BINDIR = $(HOME)/.local/bin
SOURCES = $(wildcard src/*.pony)

all: mkultra

mkultra: $(SOURCES)
	ponyc src -b mkultra -o .

clean:
	rm -f mkultra mkultra.o mkultra-test

test:
	ponyc src --debug -b mkultra-test -o . && ./mkultra-test

install: $(BINDIR)/mkultra

$(BINDIR)/mkultra: mkultra
	install -Dm755 mkultra $(BINDIR)/mkultra

uninstall:
	rm -f $(BINDIR)/mkultra
