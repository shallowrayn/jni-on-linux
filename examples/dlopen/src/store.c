#include "store.h"

static int current_value = 1;

void store(int value)
{
  current_value = value;
}

int retrieve()
{
  return current_value;
}
