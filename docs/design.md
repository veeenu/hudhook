# UI concepts

The structure of the UI will be declarative in nature.
Every command can be bound to either a global or a local hotkey.

```
Dir("Top level directory", [
  Text("Hello, world!")
  Flag([0xXXXXXXXX, 0xXX, ... 0xXX], 3, "VK_F1"),
  Separator(),

  Text("Position save state 1")
  Group([
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "X: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Y: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Z: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
  ])
  Text("Position save state 2")
  Group([
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "X: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Y: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Z: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
  ])
  Text("Position save state 3")
  Group([
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "X: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Y: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
    Value("f64", [0xXXXXXXXX, 0xXX, ... 0xXX], "Z: {V:6.2} / {S:6.2}", "VK_F4", "VK_F5"),
  ])

  Value("u32", [0xXXXXXXXX, 0xXX, ... 0xXX], "Q: {V:04} / {S:04}"),
  Dir("Group of commands", [
    Flag([0xXXXXXXXX, 0xXX, ... 0xXX], 4, "VK_F2"),
    Flag([0xXXXXXXXX, 0xXX, ... 0xXX], 2, "VK_F3"),
  ])
])
```

## Global commands

- `toggle_show`: toggle between showing or hiding the HUD
- `interact`: interact with the currently highlighted widget

## Widgets

- `Dir`. An ordered list of widgets. 

  Arguments: 
  - `caption`: a string
  - `widgets`: a list of widgets

  Main interaction: enter directory.

  Global Inputs:
  - `dir_exit`: move upwards one directory, if there is one
  - `dir_next`: move down one widget
  - `dir_prev`: move up one widget

- `Flag`. An on/off flag bound to a memory address. Its value is read
  continuously from the program's memory.

  Main interaction: toggle.

  Arguments:
  - `chain`: pointer chain of the memory address
  - `bit`: which bit to toggle (0-7)
  - `hotkey`: global hotkey which will toggle this flag; optional

- `Value`. A typed value which can be saved and loaded from memory.

  Arguments:
  - `type`: the type of value. can be one of Rust's primitive numeric types.
  - `chain`: pointer chain of the memory address
  - `fmt`: format string for the output. `{V}` and `{S}` indicate respectively
    the value currently in memory and the value currently stored (defaults to 0)
    and, like in Rust, formatting specifiers can be added after a colon after
    `V` or `S` (i.e. `"{V:.2}"` for a float with 2 decimals)
  - `store`: hotkey which stores the value from memory
  - `load`: hotkey which loads the value back into memory

  Main interaction: none. The actions of storing and loading are secundary.

- `Text`. Simple static text.

- `Group`. A grouping of widgets. When highlighted, all its children behave as
  highlighted, i.e. keypresses reaching this widget are routed to all the
  children.
