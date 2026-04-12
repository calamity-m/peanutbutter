---
name: grep
---

# Grep

## Grep file contents

```
cat <@file> | grep -i <@pattern>
```

## Return only the matching portion

> Just invert the -o to -v to exclude it

```
(
# just invert the -o to -v to exclude it 
grep -o "<@pattern>" <@file:rg . --files>
)
```


