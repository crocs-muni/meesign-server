#!/bin/bash

KEY_FOLDER="key"
mkdir --parent "./${KEY_FOLDER}"

cat > "./${KEY_FOLDER}/cert.conf" << EOT
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


openssl ecparam -name prime256v1 -genkey -noout -out "./${KEY_FOLDER}/server-key.pem"
openssl req -new -x509 -key "./${KEY_FOLDER}/server-key.pem" -out "./${KEY_FOLDER}/server-cert.pem" -days 1461 -config "./${KEY_FOLDER}/cert.conf" -nodes -extensions v3_req
rm "./${KEY_FOLDER}/cert.conf"
