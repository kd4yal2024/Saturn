TARGET = saturn-lcd-setup
CXX = g++
CXXFLAGS = -std=c++17 -O2 -Wall -Wextra
GTK_FLAGS := $(shell pkg-config --cflags --libs gtk+-3.0)

all: $(TARGET)

$(TARGET): saturn-lcd-setup.cpp
	$(CXX) $(CXXFLAGS) -o $(TARGET) saturn-lcd-setup.cpp $(GTK_FLAGS)

clean:
	rm -f $(TARGET)
