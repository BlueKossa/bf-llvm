# BF Compiler with LLVM
Extremely bad BF compiler with llvm + some new things bcs why not
## BF>>
BF>> Is BF, but better!
### Procs
Procs are "functions", they are defined with a single non-alphanumeric character, and ended with the same character. After a proc has been defined it can simply be called by writing the same identifier again. This lets BF have a much more clean syntax compared to other more primtive languages, where the programmer is required to write an entire bible just to define a function.

Procs can move the pointer internally, however, when they exit the scope of the function the pointer returns to where it was before the call.
#### Example
Example of a proc called '*' which increments a byte once, and then prints it
```bf
*>+.*+[*]
```
Which would translate to BF>><< as following:
```bf
+[>+.<]
```
## Compiling the compiler
You need all the rust build tools, as well as LLVM 14 in PATH like [this](https://gitlab.com/taricorp/llvm-sys.rs#build-requirements).