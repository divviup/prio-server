#!/bin/bash -eux
kubectl run -it --rm --image=j4cob/prio-workflow-manager:latest wfm -- -input-bucket lotophagi-ingestion-bucket
