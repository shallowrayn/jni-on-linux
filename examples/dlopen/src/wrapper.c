#include "wrapper.h"
#include "dlfcn.h"
#include <stdio.h>

typedef void (*store_t)(int value);
typedef int (*retrieve_t)();

void test_store()
{
  void *libstore = dlopen("libstore.so", RTLD_LAZY);
  store_t store = dlsym(libstore, "store");
  retrieve_t retrieve = dlsym(libstore, "retrieve");

  printf("[%zu] Value: %d\n", __LINE__, retrieve());
  store(2);
  printf("[%zu] Value: %d\n", __LINE__, retrieve());
  store(3);
  printf("[%zu] Value: %d\n", __LINE__, retrieve());
}
