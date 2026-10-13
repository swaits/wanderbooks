# GnuCash as a cross-check oracle (docs/oracle.md, "Who is right"): one
# independent implementation of double entry we diff against. Its answers
# are evidence, never the spec. Pinned so every run uses the same GnuCash.
FROM debian@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      gnucash=1:5.10-0.1+b1 python3-gnucash=1:5.10-0.1+b1 \
 && rm -rf /var/lib/apt/lists/*
COPY books.py /oracle/books.py
ENTRYPOINT ["python3", "/oracle/books.py"]
