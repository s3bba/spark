## spark

Minimal launcher for everything in your `$PATH`, wayland only.

Designed for my personal use, and very WIP type of project.

On Niri window manager (or any other) you will want to disable xray blur

```
layer-rule {
    match namespace="^spark$"
    match layer="overlay"

    background-effect {
        xray false
    }
}
```

### It currently looks like this

![Spark showcase](./showcase.png)
