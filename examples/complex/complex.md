---
tags:
    - complex
    - multi-line
---

# Complex

Multi-line snippets are supported easily. It's bash/shell/whatever, so if it'll work there, it'll work here.

## Create a dockerfile for nginx

**Use-case:**

- Hosting static files
- Creating a basic reverse proxy

**Assumptions**

There is a `public/` dir in the current working directory.

```
cat << EOF > <@dockerfile_name:?Dockerfile>
FROM nginx:alpine
COPY public /usr/nginx/html
EOF
```

## Curl with headers

```
curl -X <@http_method:echo "GET\nPOST\nPUT"> \
     -H '<@header:?Authorization>: Bearer <@value>' \
     <@url:?https://> 
```

