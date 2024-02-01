gcc -shared -fPIC math.c -o libmath.so
gcc -shared -fPIC power.c -o libpower.so -lmath -L.
gcc main.c -o main -lpower -lmath -L. -Wl,-rpath .
# ./main; echo $?
