module github.com/abetterinternet/prio-server/task-replayer

go 1.16

require github.com/letsencrypt/prio-server/workflow-manager v0.0.0-00010101000000-000000000000

replace github.com/letsencrypt/prio-server/workflow-manager => ../workflow-manager
