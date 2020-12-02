package errors

import (
	"strings"

	k8serror "k8s.io/apimachinery/pkg/api/errors"
)

func IsTransientErr(err error) bool {
	if err == nil {
		return false
	}
	return isExceededQuotaErr(err)
}

func isExceededQuotaErr(err error) bool {
	return k8serror.IsForbidden(err) && strings.Contains(err.Error(), "exceeded quota")
}
