extern int add2(int a, int b);

int add(int a, int b)
{
  return a + b;
}

int test_add()
{
  return add(2, 2);
}

int test_add2()
{
  return add2(3, 3);
}
