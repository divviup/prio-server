package utils

// Index returns the appropriate int to use in construction of filenames
// based on whether the file creator was the "first" aka PHA server.
func Index(isFirst bool) int {
	if isFirst {
		return 0
	}
	return 1
}
