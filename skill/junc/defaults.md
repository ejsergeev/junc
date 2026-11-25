# Default input and output

A Junction can be the system default input (apps that record from
"default" get the Junction) or output (apps that play to "default" feed
it). Changing the default affects every app: ask the user first.

```sh
junc default zoom-mic --as input
junc default zoom-mic --clear
junc switch zoom-mic --as input
```

`switch` turns the Junction on and makes it the default in one go. In the
config it is `default = "input"` or `default = "output"`; only one
Junction may hold each role (`E_CONFIG_DEFAULT_DUPLICATE`).

What JUNC does:

- When a Junction becomes the default, JUNC remembers the previous default
  and puts it back when the Junction stops being the default, is turned
  off or deleted. If the previous device is gone, the system picks one.
- If the user picks another default elsewhere (system settings,
  `wpctl set-default`) while a Junction holds it, JUNC does not fight it;
  it reports `default_taken_over` and stops claiming the role.
