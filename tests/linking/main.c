#include "power.h"

int m_cube(int x)
{
  return square(x) * x;
}

int main()
{
  return test_libpower();
}
