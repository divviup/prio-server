package tokenfetcher

import (
	"fmt"
	"io/ioutil"
	"log"
	"net/http"

	"github.com/aws/aws-sdk-go/aws/credentials"
)

type tokenFetcher struct {
	audience string
}

func NewTokenFetcher(audience string) tokenFetcher {
	return tokenFetcher{audience}
}

func (tf tokenFetcher) FetchToken(credentials.Context) ([]byte, error) {
	url := fmt.Sprintf("http://metadata.google.internal:80/computeMetadata/v1/instance/service-accounts/default/identity?audience=%s", tf.audience)
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, fmt.Errorf("building request: %w", err)
	}
	req.Header.Add("Metadata-Flavor", "Google")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("fetching %s: %w", url, err)
	}
	bytes, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("reading body of %s: %w", url, err)
	}
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("status code %d from metadata service at %s: %s",
			resp.StatusCode, url, string(bytes))
	}
	log.Printf("fetched token from %s", url)
	return bytes, nil
}
