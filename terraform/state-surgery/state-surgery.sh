#!/bin/bash -eu

STATEFILE=${1}
NAMESPACES=${2}
INGESTORS=${3}
EXTRA_ARGS=${4:-}

declare -a RESOURCE_RENAMES=(
    'google_compute_network.network module.gke[0].google_compute_network.network'
    'module.gke.google_compute_router.router module.gke[0].google_compute_router.router'
    'module.gke.google_compute_router_nat.nat module.gke[0].google_compute_router_nat.nat'
    'module.gke.google_compute_subnetwork.subnet module.gke[0].google_compute_subnetwork.subnet'
    'module.gke.google_container_cluster.cluster module.gke[0].google_container_cluster.cluster'
    'module.gke.google_container_node_pool.worker_nodes module.gke[0].google_container_node_pool.worker_nodes'
    'module.gke.google_kms_crypto_key.etcd_encryption_key module.gke[0].google_kms_crypto_key.etcd_encryption_key'
    'module.gke.google_kms_crypto_key_iam_binding.etcd-encryption-key-iam-binding module.gke[0].google_kms_crypto_key_iam_binding.etcd-encryption-key-iam-binding'
    'module.gke.google_kms_key_ring.keyring module.gke[0].google_kms_key_ring.keyring'
    'google_project_service.compute module.gke[0].google_project_service.compute'
    'google_project_service.container module.gke[0].google_project_service.container'
    'google_project_service.kms module.gke[0].google_project_service.kms'
    'module.manifest.google_compute_backend_bucket.manifests module.manifest_gcp[0].google_compute_backend_bucket.manifests[0]'
    'module.manifest.google_compute_global_address.manifests module.manifest_gcp[0].google_compute_global_address.manifests[0]'
    'module.manifest.google_compute_global_forwarding_rule.manifests module.manifest_gcp[0].google_compute_global_forwarding_rule.manifests[0]'
    'module.manifest.google_compute_managed_ssl_certificate.manifests module.manifest_gcp[0].google_compute_managed_ssl_certificate.manifests[0]'
    'module.manifest.google_compute_target_https_proxy.manifests module.manifest_gcp[0].google_compute_target_https_proxy.manifests[0]'
    'module.manifest.google_compute_url_map.manifests module.manifest_gcp[0].google_compute_url_map.manifests[0]'
    'module.manifest.google_dns_record_set.manifests module.manifest_gcp[0].google_dns_record_set.manifests[0]'
    'module.manifest.google_storage_bucket.manifests module.manifest_gcp[0].google_storage_bucket.manifests'
    'module.manifest.google_storage_bucket_iam_binding.public_read module.manifest_gcp[0].google_storage_bucket_iam_binding.public_read'
    'module.manifest.google_storage_bucket_object.global_manifest module.manifest_gcp[0].google_storage_bucket_object.global_manifest'
    'module.monitoring.kubernetes_persistent_volume.prometheus_server module.monitoring.kubernetes_persistent_volume.prometheus_server_gcp[0]'
)

for RENAME in "${RESOURCE_RENAMES[@]}"; do
    terraform state mv ${EXTRA_ARGS} --state=${STATEFILE} ${RENAME}
done

for NAMESPACE in ${NAMESPACES}; do
    declare -a NAMESPACE_RESOURCE_RENAMES=(
        'module.locality_kubernetes["'${NAMESPACE}'"].google_storage_bucket_iam_member.manifest_bucket_owner module.locality_kubernetes["'${NAMESPACE}'"].google_storage_bucket_iam_member.manifest_bucket_writer[0]'
    )
    for RENAME in "${NAMESPACE_RESOURCE_RENAMES[@]}"; do
        terraform state mv ${EXTRA_ARGS} --state=${STATEFILE} ${RENAME}
    done

    for INGESTOR in ${INGESTORS}; do
        INSTANCE_NAME="${NAMESPACE}-${INGESTOR}"
        declare -a INSTANCE_RESOURCE_RENAMES=(
            'module.data_share_processors["'${INSTANCE_NAME}'"].google_kms_crypto_key.bucket_encryption module.data_share_processors["'${INSTANCE_NAME}'"].module.cloud_storage_gcp[0].google_kms_crypto_key.bucket_encryption'
            'module.data_share_processors["'${INSTANCE_NAME}'"].google_kms_crypto_key_iam_binding.bucket_encryption_key module.data_share_processors["'${INSTANCE_NAME}'"].module.cloud_storage_gcp[0].google_kms_crypto_key_iam_binding.bucket_encryption_key'
            'module.data_share_processors["'${INSTANCE_NAME}'"].module.bucket.google_storage_bucket.bucket module.data_share_processors["'${INSTANCE_NAME}'"].module.cloud_storage_gcp[0].module.bucket["own_validation"].google_storage_bucket.bucket'
            'module.data_share_processors["'${INSTANCE_NAME}'"].module.bucket.google_storage_bucket_iam_binding.bucket_reader module.data_share_processors["'${INSTANCE_NAME}'"].module.cloud_storage_gcp[0].module.bucket["own_validation"].google_storage_bucket_iam_binding.bucket_reader'
            'module.data_share_processors["'${INSTANCE_NAME}'"].module.bucket.google_storage_bucket_iam_binding.bucket_writer module.data_share_processors["'${INSTANCE_NAME}'"].module.cloud_storage_gcp[0].module.bucket["own_validation"].google_storage_bucket_iam_binding.bucket_writer'
            'module.data_share_processors["'${INSTANCE_NAME}'"].module.kubernetes.google_service_account_iam_binding.workflow_manager_token module.data_share_processors["'${INSTANCE_NAME}'"].module.kubernetes.module.account_mapping.google_service_account_iam_binding.workflow_manager_token[0]'
        )
        for RENAME in "${INSTANCE_RESOURCE_RENAMES[@]}"; do
            terraform state mv ${EXTRA_ARGS} --state=${STATEFILE} ${RENAME}
        done
    done
done
