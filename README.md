
---

# lc2llvm

<p align="left">
  <img src="https://img.shields.io/badge/compiler-lambda→llvm-black?style=flat-square" />
  <img src="https://img.shields.io/badge/status-experimental-grey?style=flat-square" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" />
</p>

A minimal compiler that translates **Lambda Calculus expressions** into **LLVM IR**, exploring the connection between formal computation and modern compiler infrastructure.

---

## Overview

`lc2llvm` implements a simple compilation pipeline:

```
Lambda Calculus → AST → (optional reduction) → LLVM IR
```

It is intended for experimentation, learning, and as a foundation for building more advanced functional compilers.

---

## Features

* Lambda calculus parser (abstraction, application, variables)
* AST construction
* LLVM IR generation
* Simple and modular pipeline
* Easy to extend

---

## Build

### Requirements

* LLVM (≥ 14)
* CMake + compiler toolchain

### Steps

```bash
git clone https://github.com/vinayydv3695/lc2llvm.git
cd lc2llvm

mkdir build && cd build
cmake ..
make
```

---

## Usage

### Input

Create a `.lc` file:

```
(λx. x) 5
```

### Compile

```bash
./lc2llvm input.lc -o output.ll
```

### Run

```bash
lli output.ll
```

or:

```bash
clang output.ll -o program
./program
```

---

## Example

**Input**

```
λf.λx.f (f x)
```

**Output (simplified)**

```llvm
define i32 @main() {
entry:
  ret i32 0
}
```

---

## Project Structure

```
src/
  lexer/
  parser/
  ast/
  codegen/
  main.*
```

---

## Roadmap

* Type system (STLC)
* Optimizations (inlining, constant folding)
* REPL
* Better error handling
* JIT support

---

## Contributing

PRs are welcome. Keep changes minimal and well-structured.

---

## License

MIT

---

## Author

Vinay Yadav
[https://github.com/vinayydv3695](https://github.com/vinayydv3695)

