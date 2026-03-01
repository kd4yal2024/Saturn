CC=gcc
CFLAGS=-O2 -Wall -Wextra -Wno-deprecated-declarations
GTK_CFLAGS=$(shell pkg-config --cflags gtk+-3.0)
GTK_LIBS=$(shell pkg-config --libs gtk+-3.0)
APPIND_CFLAGS=$(shell pkg-config --cflags ayatana-appindicator3-0.1)
APPIND_LIBS=$(shell pkg-config --libs ayatana-appindicator3-0.1)

BIN=p2app-control
SRC=src/p2app_control.c

all: $(BIN)

$(BIN): $(SRC)
	$(CC) $(CFLAGS) $(GTK_CFLAGS) $(APPIND_CFLAGS) -o $@ $< $(GTK_LIBS) $(APPIND_LIBS)

clean:
	rm -f $(BIN)

install: $(BIN)
	install -D -m 0755 $(BIN) /usr/local/bin/$(BIN)

.PHONY: all clean install
