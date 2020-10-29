#!/bin/bash -eux
N="-n jsha-peer-1"
J=peer-1-baku-manual
kubectl $N delete job $J
kubectl $N create job --from=cronjob/jsha-fridemo-jsha-peer-1-baku-workflow-manager $J 
sleep 10
kubectl $N logs --follow jobs/$J
