---
tags:
    - curl
    - dockerfile
    - docker
---

# Complex

## Create a dockerfile for nginx

Create a minimal nginx dockerfile, which is good for:

- Hosting static files
- Creating a basic reverse proxy


```
cat << EOF > <@dockerfile_name:?Dockerfile>
FROM nginx:alpine
COPY public /usr/nginx/html
EOF
```

## Curl with headers


curl


### Header Syntax

**"Header: Value"**

```
curl -X <@http_method:echo "GET\nPOST\nPUT"> \
     -H '<@header:?Authorization>: <@value:?Bearer ...>' \
     <@url:?https://...> 
```

