package secrets

//
//import (
//	"context"
//	"crypto/ecdsa"
//	"crypto/elliptic"
//	"crypto/rand"
//	"crypto/x509"
//	"encoding/base64"
//	"encoding/pem"
//	"fmt"
//	"github.com/abetterinternet/prio-server/manifest-updater/manifest"
//	corev1 "k8s.io/api/core/v1"
//	v1 "k8s.io/apimachinery/pkg/apis/meta/v1"
//	"time"
//)
//
//const (
//	batchSigningKeyExpirationDuration = 90 * 24 * time.Hour
//
//	batchSigningKeyMaxAge = 70 * 24 * time.Hour
//)
//
//func (k *Kube) validateAndUpdateBatchSigningKey(keyName, dsp string, secret *corev1.Secret) (*KubernetesBatchSigningKey, error) {
//	creation := secret.GetCreationTimestamp()
//	since := time.Since(creation.Time)
//
//	expired := since > batchSigningKeyMaxAge
//
//	if !expired {
//		return nil, nil
//	}
//
//	k.log.
//		WithField("KeyType", "BatchSigningKey").
//		WithField("Expiration: ", expired).
//		Info("Secret is close to expiration or has expired, we're going to require it to be expired")
//
//	cert, err := k.createAndStoreBatchSigningKey(keyName, dsp)
//
//	if err != nil {
//		return nil, fmt.Errorf("unable to create secret: %w", err)
//	}
//
//	cert.PreviousKubernetesIdentifier = secret.Name
//	return cert, nil
//}
//
//func (k *Kube) createAndStoreBatchSigningKey(name, dsp string) (*KubernetesBatchSigningKey, error) {
//	privKey, pubKey, err := createBatchSigningKey()
//
//	if err != nil {
//		return nil, fmt.Errorf("unable to create a batch signing key: %w", err)
//	}
//
//	immutable := true
//
//	secret := corev1.Secret{
//		ObjectMeta: v1.ObjectMeta{
//			GenerateName: name,
//			Namespace:    k.namespace,
//			Labels: map[string]string{
//				"type": "batch-signing-key",
//				"dsp":  dsp,
//			},
//		},
//		Immutable: &immutable,
//
//		StringData: map[string]string{
//			secretKeyMap: privKey,
//		},
//	}
//
//	expiration := time.
//		Now().
//		Add(batchSigningKeyExpirationDuration).
//		UTC().
//		Format(time.RFC3339)
//
//	sApi := k.client.CoreV1().Secrets(k.namespace)
//	created, err := sApi.Create(context.Background(), &secret, v1.CreateOptions{})
//
//	if err != nil {
//		return nil, fmt.Errorf("failed to store secret %w", err)
//	}
//
//	return &KubernetesBatchSigningKey{
//		Key: &manifest.BatchSigningPublicKey{
//			PublicKey:  pubKey,
//			Expiration: expiration,
//		},
//		KubernetesIdentifier: created.Name,
//	}, nil
//}
//
//func createBatchSigningKey() KubernetesBatchSigningKey {
//	p256Key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
//	if err != nil {
//		return "", "", fmt.Errorf("failed to generate P-256 key: %w", err)
//	}
//	pkixPublic, err := x509.MarshalPKIXPublicKey(p256Key.Public())
//	if err != nil {
//		return "", "", fmt.Errorf("failed to marshal ECDSA public key to PKIX %w", err)
//	}
//
//	block := &pem.Block{
//		Type:  "PUBLIC KEY",
//		Bytes: pkixPublic,
//	}
//
//	marshaledPrivateKey, err := marshalX962UncompressedPrivateKey(p256Key)
//	if err != nil {
//		return "", "", fmt.Errorf("failed to marshall key to PKCS#8 document: %w", err)
//	}
//
//	base64PrivateKey := base64.StdEncoding.EncodeToString(marshaledPrivateKey)
//	return base64PrivateKey, string(pem.EncodeToMemory(block)), nil
//}
