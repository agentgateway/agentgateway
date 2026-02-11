ARG BASE_IMAGE

FROM $BASE_IMAGE

ARG GOARCH=amd64

RUN apk -U upgrade

COPY dummy-idp-linux-$GOARCH /usr/local/bin/dummy-idp

USER 10101

ENTRYPOINT ["/usr/local/bin/dummy-idp"]
