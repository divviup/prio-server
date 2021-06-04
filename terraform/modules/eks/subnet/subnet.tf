variable "resource_prefix" {
  type = string
}

variable "availability_zone_id" {
  type = string
}

variable "availability_zone_name" {
  type = string
}

variable "vpc_id" {
  type = string
}

variable "subnet_cidr" {
  type = string
}

variable "public_subnet" {
  type = string
}

resource "aws_subnet" "subnet" {
  availability_zone_id = var.availability_zone_id
  cidr_block           = var.subnet_cidr
  vpc_id               = var.vpc_id

  tags = {
    Name = "${var.resource_prefix}-${var.availability_zone_name}"
  }
}

# Elastic (public) IP to be attached to the NAT gateway
resource "aws_eip" "internet_egress" {
  vpc = true

  tags = {
    Name = "${var.resource_prefix}-${var.availability_zone_name}-internet-egress"
  }
}

# The VPC has an Internet gateway, but since all our subnets are private, we
# need a NAT gateway through which worker nodes will reach the Internet. Though
# the NAT gateway allows Internet egress for the private subnet, it must be
# created in the public subnet.
resource "aws_nat_gateway" "internet_egress" {
  allocation_id = aws_eip.internet_egress.id
  subnet_id     = var.public_subnet

  tags = {
    Name = "${var.resource_prefix}-${var.availability_zone_name}-internet-egress"
  }
}

# Configure a route table for the subnet that allows egress to the Internet
resource "aws_route_table" "internet_egress" {
  vpc_id = var.vpc_id

  # The route table has an implicit rule which routes traffic to the VPC's
  # CIDR locally, and it will have higher precedence than the rule we specify
  # here for Internet egress, permitting intra-cluster communication.
  # https://docs.aws.amazon.com/vpc/latest/userguide/VPC_Route_Tables.html#how-route-tables-work
  route {
    cidr_block     = "0.0.0.0/0"
    nat_gateway_id = aws_nat_gateway.internet_egress.id
  }

  tags = {
    Name = "${var.resource_prefix}-${var.availability_zone_name}-internet-egress"
  }
}

resource "aws_route_table_association" "internet_egress" {
  subnet_id      = aws_subnet.subnet.id
  route_table_id = aws_route_table.internet_egress.id
}

output "id" {
  value = aws_subnet.subnet.id
}

output "route_table_id" {
  value = aws_route_table.internet_egress.id
}


