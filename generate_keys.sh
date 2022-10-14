#!/bin/bash

KEY_FOLDER="key"
mkdir --parent "./${KEY_FOLDER}"

# MeeSign CA certificate configuration
cat > ca-cert.conf << EOT
[req]
distinguished_name = req_distinguished_name
req_extensions = v3_req
prompt = no
utf8 = yes

[req_distinguished_name]
C = CS
O = MeeSign
CN = MeeSign CA

[v3_req]
basicConstraints = critical, CA:TRUE
authorityKeyIdentifier = keyid, issuer
subjectKeyIdentifier = hash
keyUsage = critical, cRLSign, digitalSignature, keyCertSign
EOT

# MeeSign server certificate configuration
cat > server-csr.conf << EOT
[req]
distinguished_name = req_distinguished_name
prompt = no
utf8 = yes

[req_distinguished_name]
C = CS
O = MeeSign
CN = meesign.local
EOT

# Standard server X509v3 extensions
cat > server-ext.conf << EOT
basicConstraints = critical, CA:FALSE
authorityKeyIdentifier = keyid, issuer
subjectKeyIdentifier = hash
keyUsage = critical, nonRepudiation, digitalSignature, keyEncipherment, keyAgreement
extendedKeyUsage = critical, serverAuth
EOT

# Generate MeeSign CA private key
openssl ecparam -name prime256v1 -genkey -noout -out "./${KEY_FOLDER}/meesign-ca-key.pem"
# Issue self-signed certificate for MeeSign CA
openssl req -new -x509 -key "./${KEY_FOLDER}/meesign-ca-key.pem" -out "./${KEY_FOLDER}/meesign-ca-cert.pem" -days 1461 -config ca-cert.conf -nodes -extensions v3_req

# Generate MeeSign server private key
openssl ecparam -name prime256v1 -genkey -noout -out "./${KEY_FOLDER}/meesign-server-key-ec.pem"
openssl pkcs8 -topk8 -nocrypt -in "./${KEY_FOLDER}/meesign-server-key-ec.pem" -out "./${KEY_FOLDER}/meesign-server-key.pem"
rm "./${KEY_FOLDER}/meesign-server-key-ec.pem"
# Create certificate signing request for MeeSign server certificate
openssl req -new -key "./${KEY_FOLDER}/meesign-server-key.pem" -out csr.pem -config server-csr.conf
# Sign MeeSign server certificate signing request by MeeSign CA
openssl x509 -req -days 365 -in csr.pem -CA "./${KEY_FOLDER}/meesign-ca-cert.pem" -CAkey "./${KEY_FOLDER}/meesign-ca-key.pem" -CAcreateserial -out "./${KEY_FOLDER}/meesign-server-cert.pem" -extfile server-ext.conf

rm ca-cert.conf server-csr.conf server-ext.conf csr.pem
