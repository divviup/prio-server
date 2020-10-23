package gcloud

import (
	"context"
	"fmt"
	"time"

	"github.com/libdns/libdns"
	"golang.org/x/oauth2/google"
	"google.golang.org/api/dns/v1"
	"google.golang.org/api/option"
)

// GoogleDNSProvider is a structure that defines the google DNS provider implementation
type GoogleDNSProvider struct {
	service      *dns.Service
	project      string
	zoneMappings map[string]string
}

const (
	changeStatusDone = "done"
)

// NewGoogleDNSProvider creates a new GoogleDNSProvider with the given project name
func NewGoogleDNSProvider(project string, zoneMappings map[string]string) (*GoogleDNSProvider, error) {
	client, err := google.DefaultClient(context.Background(), dns.NdevClouddnsReadwriteScope)
	if err != nil {
		return nil, err
	}

	svc, err := dns.NewService(context.Background(), option.WithHTTPClient(client))
	if err != nil {
		return nil, err
	}

	return &GoogleDNSProvider{
		service:      svc,
		project:      project,
		zoneMappings: zoneMappings,
	}, nil
}

func convertRecordType(record libdns.Record) dns.ResourceRecordSet {
	return dns.ResourceRecordSet{
		Name:    fmt.Sprintf("%s.", record.Name),
		Rrdatas: []string{record.Value},
		Ttl:     record.TTL.Nanoseconds() / 1e9, // Expected in seconds
		Type:    record.Type,
		Kind:    "dns#resourceRecordSet",
	}
}

func (p *GoogleDNSProvider) convertZone(zone string) string {
	newZone, ok := p.zoneMappings[zone]
	if ok {
		return newZone
	}

	return zone
}

// AppendRecords appends DNS records to a given zone
func (p *GoogleDNSProvider) AppendRecords(ctx context.Context, zone string, recs []libdns.Record) ([]libdns.Record, error) {
	zone = p.convertZone(zone)
	gcpRecords := make([]*dns.ResourceRecordSet, len(recs))

	for idx, record := range recs {
		resourceRecordSet := convertRecordType(record)
		gcpRecords[idx] = &resourceRecordSet
	}
	changes := &dns.Change{
		Additions: gcpRecords,
	}
	result, err := p.service.Changes.Create(p.project, zone, changes).Do()
	if err != nil {
		return nil, err
	}

	if result.Status == changeStatusDone {
		return recs, nil
	}

	changeID := result.Id
	err = poll("append records", 30*time.Second, 3*time.Second, func() (bool, error) {
		result, err := p.service.Changes.Get(p.project, zone, changeID).Do()
		if err != nil {
			return false, err
		}

		if result.Status == changeStatusDone {
			return true, nil
		}
		return false, nil
	})

	if err != nil {
		return nil, err
	}

	return recs, nil
}

// DeleteRecords deletes DNS records from a given zone
func (p *GoogleDNSProvider) DeleteRecords(ctx context.Context, zone string, recs []libdns.Record) ([]libdns.Record, error) {
	zone = p.convertZone(zone)
	gcpRecords := make([]*dns.ResourceRecordSet, len(recs))

	for idx, record := range recs {
		resourceRecordSet := convertRecordType(record)
		gcpRecords[idx] = &resourceRecordSet
	}
	changes := &dns.Change{
		Deletions: gcpRecords,
	}
	result, err := p.service.Changes.Create(p.project, zone, changes).Do()
	if err != nil {
		return nil, err
	}

	if result.Status == changeStatusDone {
		return recs, nil
	}

	changeID := result.Id
	err = poll("delete records", 30*time.Second, 3*time.Second, func() (bool, error) {
		result, err := p.service.Changes.Get(p.project, zone, changeID).Do()
		if err != nil {
			return false, err
		}

		if result.Status == changeStatusDone {
			return true, nil
		}
		return false, nil
	})

	if err != nil {
		return nil, err
	}

	return recs, nil
}

// poll is a blocking polling for a given function with a given name.
// The poller must return true for the polling code to stop.
// The poller may also return an optional error to be reported
func poll(job string, timeout, interval time.Duration, poller func() (bool, error)) error {
	var lastErr error
	timeUp := time.After(timeout)

	for {
		select {
		case <-timeUp:
			return fmt.Errorf("job %s timed out. Last error: %w", job, lastErr)
		default:
		}

		stop, err := poller()

		if stop {
			return nil
		}

		if err != nil {
			lastErr = err
		}

		time.Sleep(interval)
	}
}
