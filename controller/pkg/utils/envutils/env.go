package envutils

import (
	"os"
	"strconv"
)

// IsEnvTruthy returns true if a given environment variable has a truthy value
// Examples of truthy values are: "1", "t", "T", "true", "TRUE", "True". Anything else is considered false.
func IsEnvTruthy(envVarName string) bool {
	return IsTruthyValue(os.Getenv(envVarName))
}

// IsEnvDefined returns true if a given environment variable has any value
func IsEnvDefined(envVarName string) bool {
	envValue := os.Getenv(envVarName)
	return len(envValue) > 0
}

// IsTruthyValue returns true if a given value is "truthy".
// Examples of truthy values are: "1", "t", "T", "true", "TRUE", "True". Anything else is considered false.
func IsTruthyValue(value string) bool {
	envValue, _ := strconv.ParseBool(value)
	return envValue
}

// GetOrDefault returns the value of the environment variable for the given key,
// or the default value if the environment variable is not set. A value of "" will
// only be returned if allowEmpty is true. Otherwise, the empty value is ignored and
// the default is returned.
func GetOrDefault(key, fallback string, allowEmpty bool) string {
	if value, ok := os.LookupEnv(key); ok {
		if allowEmpty || len(value) > 0 {
			return value
		}
	}
	return fallback
}

// LookupOrDefault returns the value of the environment variable for the given key,
// or the default value if the environment variable is not set. Also returns whether
// the value existed.
func LookupOrDefault(key, fallback string) (string, bool) {
	if value, ok := os.LookupEnv(key); ok {
		return value, ok
	} else {
		return fallback, ok
	}
}
