#include "power.h"
#include "math.h"

// math.h doesn't support this yet
extern int m_cube(int x);

int square(int x)
{
  return m_square(x);
}

int cube(int x)
{
  return m_cube(x);
}

int test_libpower()
{
  int x = 4;
  if (square(x) != 16)
  {
    return 1;
  }
  if (cube(x) != 64)
  {
    return 2;
  }
  return 0;
}
