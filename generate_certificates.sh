#!/bin/bash
cat > cert.conf << EOT
[req]
distinguished_name = req_distinguished_name
req_extensions = v3_req
prompt = no
utf8 = yes

[req_distinguished_name]
C = CS
O = MeeSign
CN = MeeSign

[v3_req]
basicConstraints = critical, CA:TRUE
authorityKeyIdentifier = keyid, issuer
subjectKeyIdentifier = hash
keyUsage = critical, cRLSign, digitalSignature, keyCertSign
EOT

openssl ecparam -name prime256v1 -genkey -noout -out server-key.pem
openssl req -new -x509 -key server-key.pem -out server-cert.pem -days 1461 -config cert.conf -nodes -extensions v3_req
rm cert.conf
