#!/bin/bash -eux
cd $(dirname $0)
docker tag letsencrypt/prio-workflow-manager j4cob/prio-workflow-manager:latest
docker push j4cob/prio-workflow-manager
