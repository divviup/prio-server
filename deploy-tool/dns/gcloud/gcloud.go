package gcloud

import (
	"context"
	"fmt"
	"github.com/libdns/libdns"
	"golang.org/x/oauth2/google"
	"google.golang.org/api/dns/v1"
	"google.golang.org/api/option"
	"time"
)

type Provider struct {
	service *dns.Service
	project string
}

const (
	changeStatusDone = "done"
)

func NewProvider(project string) (*Provider, error) {
	client, err := google.DefaultClient(context.Background(), dns.NdevClouddnsReadwriteScope)
	if err != nil {
		return nil, err
	}

	svc, err := dns.NewService(context.Background(), option.WithHTTPClient(client))
	if err != nil {
		return nil, err
	}

	return &Provider{
		service: svc,
		project: project,
	}, nil
}

func convertRecordType(record libdns.Record) dns.ResourceRecordSet {
	return dns.ResourceRecordSet{
		Name:    record.Name,
		Rrdatas: []string{record.Value},
		Ttl:     record.TTL.Nanoseconds() / 1e9, // Expected in seconds
		Type:    record.Type,
	}
}

func (p *Provider) AppendRecords(ctx context.Context, zone string, recs []libdns.Record) ([]libdns.Record, error) {
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

	changeId := result.Id
	err = Poll("append records", 30*time.Second, 3*time.Second, func() (bool, error) {
		result, err := p.service.Changes.Get(p.project, zone, changeId).Do()
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

func (p *Provider) DeleteRecords(ctx context.Context, zone string, recs []libdns.Record) ([]libdns.Record, error) {
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

	changeId := result.Id
	err = Poll("delete records", 30*time.Second, 3*time.Second, func() (bool, error) {
		result, err := p.service.Changes.Get(p.project, zone, changeId).Do()
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

func Poll(job string, timeout, interval time.Duration, poller func() (bool, error)) error {
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
