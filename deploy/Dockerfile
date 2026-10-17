FROM debian:trixie-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
COPY build/imboy-textguard /usr/local/bin/imboy-textguard
RUN mkdir -p /var/lib/textguard && chown 10001:10001 /var/lib/textguard && chmod 700 /var/lib/textguard
USER 10001:10001
ENV TEXTGUARD_DATA_DIR=/var/lib/textguard
ENV TEXTGUARD_CHECK_SECRET_FILE=/run/secrets/check
ENV TEXTGUARD_PUBLISH_SECRET_FILE=/run/secrets/publish
HEALTHCHECK --interval=10s --timeout=2s --start-period=2s --retries=3 CMD ["/usr/local/bin/imboy-textguard", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/imboy-textguard"]
